#!/usr/bin/env bash
# Real-nginx end-to-end check. Run from the repo root:
#   cargo build --release && docker run --rm -v $PWD/target/release/ngx:/usr/local/bin/ngx:ro -v $PWD/tests/e2e/arch.sh:/e2e.sh:ro archlinux:latest bash /e2e.sh
set -e
pacman -Sy --noconfirm --needed nginx nodejs >/dev/null 2>&1
nginx && echo "nginx started"
cd /tmp && node -e 'require("http").createServer((q,r)=>r.end("hello from node\n")).listen(3000,"127.0.0.1")' & sleep 1
echo "=== doctor"; ngx doctor || true
echo "=== init (arch layout)"; ngx init -y
grep -n sites-enabled /etc/nginx/nginx.conf
echo "=== add"; ngx add -y -d app.test -l 3000 --ssl none | tail -2
curl -s -H 'Host: app.test' http://127.0.0.1/
echo "=== add static"; ngx add -y -d www.static.test --root /srv/static --ssl none | tail -1
curl -s -H 'Host: www.static.test' http://127.0.0.1/ | head -c 80; echo
echo "=== hardening"
mkdir -p /srv/static/.git && echo secret > /srv/static/.git/config && echo SECRET=1 > /srv/static/.env
echo "hidden files (expect 403 403):" $(curl -s -o /dev/null -w '%{http_code}' -H 'Host: www.static.test' http://127.0.0.1/.git/config) \
  $(curl -s -o /dev/null -w '%{http_code}' -H 'Host: www.static.test' http://127.0.0.1/.env)
ngx add -y -d php.test -t php --root /srv/php --socket /run/php-fpm/php-fpm.sock --ssl none | tail -1
grep -q 'try_files $fastcgi_script_name =404' /etc/nginx/sites-available/php.test.conf && echo "php: script must exist (ok)"
echo "php hidden file (expect 403):" $(curl -s -o /dev/null -w '%{http_code}' -H 'Host: php.test' http://127.0.0.1/.env.php)
ngx rm -y php.test | tail -1
ngx add -y -d 'evil.test; include /etc/shadow' -l 3000 --ssl none 2>&1 | tail -1
ngx add -y -d evil.test -t static --root / --ssl none 2>&1 | tail -1
ls /etc/nginx/sites-available | grep -c evil || echo "no evil config written (ok)"
echo "=== ls"; ngx ls
echo "=== show"; ngx show 1
echo "=== edit via prompts/flags: move app to port 3001"
node -e 'require("http").createServer((q,r)=>r.end("hello from 3001\n")).listen(3001,"127.0.0.1")' & sleep 1
ngx edit 1 -y --local 3001 | tail -2
curl -s -H 'Host: app.test' http://127.0.0.1/
echo "=== rename domain keeps ID"; ngx edit 1 -y -d app2.test | tail -1; ngx ls | grep app2
curl -s -H 'Host: app2.test' http://127.0.0.1/
echo "=== broken raw edit must roll back"
EDITOR="sed -i s/proxy_pass/proxy_pas/" ngx edit 1 --raw </dev/null || echo "(edit rejected as expected)"
grep -c 'proxy_pass ' /etc/nginx/sites-available/app2.test.conf
curl -s -H 'Host: app2.test' http://127.0.0.1/
echo "=== stop backend -> down"; pkill node; sleep 0.5; ngx ls | grep app2.test
ngx show app2.test | sed -n '/BACKEND/,/^$/p;/RECENT/,/^$/p'
curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: app2.test' http://127.0.0.1/
echo "=== duplicate via nginx -t"; printf 'server { listen 80; server_name x.test; bogus_directive on; }\n' > /etc/nginx/sites-available/bad.conf; ln -s /etc/nginx/sites-available/bad.conf /etc/nginx/sites-enabled/bad.conf
ngx ls | grep -E 'x.test|nginx:' ; ngx test || true
rm /etc/nginx/sites-enabled/bad.conf
echo "=== disable / reload"; ngx disable 2 && curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: www.static.test' http://127.0.0.1/
