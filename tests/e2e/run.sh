#!/usr/bin/env bash
# End-to-end test with a real nginx, in a throwaway container (Arch, Debian or Ubuntu).
# Run from the repo root (see docs/development.md):
#
#   cargo build --release --target x86_64-unknown-linux-musl
#   tests/e2e/docker.sh archlinux:latest      # or debian:stable, ubuntu:24.04
#
# Every check prints PASS/FAIL; the script exits non-zero if any check failed.
set -u
FAILS=0
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; FAILS=$((FAILS + 1)); }
check() { local what=$1; shift; if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi; }
code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
eq() { [ "$1" = "$2" ] && pass "$3 ($1)" || fail "$3 (got '$1', expected '$2')"; }
section() { echo; echo "=== $*"; }

section "install"
if command -v pacman >/dev/null; then
  pacman -Sy --noconfirm --needed nginx nodejs openssl procps-ng >/dev/null 2>&1
  DISTRO=$(. /etc/os-release; echo "$ID")
else
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq >/dev/null && apt-get install -y -qq nginx nodejs curl openssl procps >/dev/null 2>&1
  DISTRO=$(. /etc/os-release; echo "$ID")
  rm -f /etc/nginx/sites-enabled/default
fi
nginx -v 2>&1
nginx && pass "nginx started"
serve() { (cd /tmp && exec node -e "require('http').createServer((q,r)=>r.end('$2 '+q.url+'\n')).listen($1,'127.0.0.1')") & }
serve 3000 app; serve 3001 app3001; serve 8080 api; sleep 1

section "doctor / init"
ngx doctor || true
ngx init -y | tail -1
check "nginx.conf includes sites-enabled" grep -q sites-enabled /etc/nginx/nginx.conf

section "add proxy + static"
ngx add -y -d app.test -l 3000 --ssl none | tail -1
eq "$(curl -s -H 'Host: app.test' http://127.0.0.1/x)" "app /x" "proxy serves the app"
ngx add -y -d www.static.test --root /srv/static --ssl none | tail -1
check "static index created" curl -sf -H 'Host: www.static.test' http://127.0.0.1/

section "hardening"
mkdir -p /srv/static/.git && echo secret > /srv/static/.git/config && echo SECRET=1 > /srv/static/.env
eq "$(code -H 'Host: www.static.test' http://127.0.0.1/.git/config)" 403 "hidden .git denied"
eq "$(code -H 'Host: www.static.test' http://127.0.0.1/.env)" 403 "hidden .env denied"
ngx add -y -d php.test -t php --root /srv/php --socket /run/php-fpm/php-fpm.sock --ssl none | tail -1
check "php only runs existing scripts" grep -q 'try_files $fastcgi_script_name =404' /etc/nginx/sites-available/php.test.conf
eq "$(code -H 'Host: php.test' http://127.0.0.1/.env.php)" 403 "hidden .php not executed"
ngx rm -y php.test | tail -1
! ngx add -y -d 'evil.test; include /etc/shadow' -l 3000 --ssl none >/dev/null 2>&1 && pass "injected domain rejected" || fail "injected domain rejected"
! ngx add -y -d evil.test -t static --root / --ssl none >/dev/null 2>&1 && pass "/ as document root rejected" || fail "/ as document root rejected"
check "no config written for rejected input" bash -c '! ls /etc/nginx/sites-available | grep -q evil'

section "options: upload size, rate limit, routes, canonical redirect, password"
ngx edit app.test -y -a www.app.test --max-body 1k --rate-limit 2r/s --route /api=8080 --canonical on | tail -1
eq "$(curl -s -H 'Host: app.test' http://127.0.0.1/api/v1)" "api /api/v1" "/api/ routed to :8080"
eq "$(code -H 'Host: app.test' -X POST --data-binary @<(head -c 4096 /dev/zero) http://127.0.0.1/)" 413 "upload over 1k refused"
eq "$(curl -s -o /dev/null -w '%{redirect_url}' -H 'Host: www.app.test' http://127.0.0.1/p?q=1)" "http://app.test/p?q=1" "alias redirects to the main domain"
codes=$(for i in $(seq 1 30); do code -H 'Host: app.test' http://127.0.0.1/; echo; done | sort | uniq -c)
echo "$codes" | grep -q 429 && pass "rate limit answers 429 on bursts" || fail "rate limit answers 429 on bursts: $codes"
ngx edit app.test -y --rate-limit off --auth on | tail -1
eq "$(code -H 'Host: app.test' http://127.0.0.1/)" 401 "password required"
echo 'correct horse battery' | ngx auth add app.test alice --password-stdin | tail -1
eq "$(code -u alice:'correct horse battery' -H 'Host: app.test' http://127.0.0.1/)" 200 "password accepted (sha512-crypt)"
eq "$(code -u alice:wrong -H 'Host: app.test' http://127.0.0.1/)" 401 "wrong password refused"
check "htpasswd not world-readable" bash -c '[ $(( 0$(stat -c %a /etc/nginx/ngx.d/app.test/htpasswd) & 4 )) -eq 0 ]'
ngx edit app.test -y --auth off --max-body off --route off --canonical off -a none | tail -1

section "snippets"
EDITOR="sh -c 'echo \"location = /health { return 200 \\\"ok\\n\\\"; }\" > \$1' sh" ngx edit app.test --snippet health </dev/null | tail -1
eq "$(curl -s -H 'Host: app.test' http://127.0.0.1/health)" "ok" "snippet is live"
ngx edit app.test -y --local 3001 | tail -1
eq "$(curl -s -H 'Host: app.test' http://127.0.0.1/health)" "ok" "snippet kept after regeneration"
EDITOR="sh -c 'echo \"bogus_directive on;\" > \$1' sh" ngx edit app.test --snippet broken </dev/null >/dev/null 2>&1
check "broken snippet rejected" bash -c '! ls /etc/nginx/ngx.d/app.test | grep -q broken'

section "HTTPS with an existing certificate + HSTS"
mkdir -p /etc/ssl/ngx-test && openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj /CN=tls.test -addext subjectAltName=DNS:tls.test \
  -keyout /etc/ssl/ngx-test/key.pem -out /etc/ssl/ngx-test/cert.pem >/dev/null 2>&1 && chmod 600 /etc/ssl/ngx-test/key.pem
ngx add -y -d tls.test -l 3000 --ssl existing --cert /etc/ssl/ngx-test/cert.pem --key /etc/ssl/ngx-test/key.pem --hsts on | tail -1
eq "$(curl -sk --resolve tls.test:443:127.0.0.1 https://tls.test/)" "app /" "HTTPS serves the app"
check "HSTS header sent" bash -c "curl -skI --resolve tls.test:443:127.0.0.1 https://tls.test/ | grep -qi strict-transport-security"
eq "$(code -H 'Host: tls.test' http://127.0.0.1/a)" 301 "HTTP redirects to HTTPS"

section "edit, rename, raw edit rollback"
ngx edit app.test -y -d app2.test | tail -1
eq "$(curl -s -H 'Host: app2.test' http://127.0.0.1/health)" "ok" "rename moved the snippets"
EDITOR="sed -i s/proxy_pass/proxy_pas/" ngx edit app2.test --raw </dev/null >/dev/null 2>&1
check "broken raw edit rolled back" grep -q 'proxy_pass ' /etc/nginx/sites-available/app2.test.conf

section "history / undo"
ngx history -n 5
before=$(curl -s -H 'Host: app2.test' http://127.0.0.1/)
ngx edit app2.test -y --local 3000 | tail -1
eq "$(curl -s -H 'Host: app2.test' http://127.0.0.1/)" "app /" "edit applied"
ngx undo -y | tail -1
eq "$(curl -s -H 'Host: app2.test' http://127.0.0.1/)" "$before" "undo restored the previous config"
check "backups are root-only" bash -c '[ "$(stat -c %a /var/lib/ngx/backups)" = 700 ]'

section "check (monitoring)"
ngx check; eq "$?" 0 "check: all good"
pkill -f "listen\(3001" ; sleep 0.5
ngx check; eq "$?" 2 "check: backend down is critical"
ngx check --json | grep -q '"critical"' && pass "check --json" || fail "check --json"

section "nginx -t errors and locking"
printf 'server { listen 80; server_name x.test; bogus_directive on; }\n' > /etc/nginx/sites-available/bad.conf
ln -s /etc/nginx/sites-available/bad.conf /etc/nginx/sites-enabled/bad.conf
ngx ls | grep -q 'unknown directive' && pass "nginx -t error mapped to the site" || fail "nginx -t error mapped to the site"
rm /etc/nginx/sites-enabled/bad.conf
(flock /var/lib/ngx/lock sleep 2) & sleep 0.3
start=$(date +%s); ngx disable www.static.test >/dev/null; end=$(date +%s)
[ $((end - start)) -ge 1 ] && pass "changes wait for the lock" || fail "changes wait for the lock"
check "disabled site no longer served" bash -c "! curl -s -H 'Host: www.static.test' http://127.0.0.1/ | grep -q 'configured by ngx'"

echo
if [ $FAILS -eq 0 ]; then echo "ALL PASSED ($DISTRO)"; else echo "$FAILS FAILED ($DISTRO)"; exit 1; fi
