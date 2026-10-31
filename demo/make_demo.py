#!/usr/bin/env python3
"""Build a sandbox filesystem that mirrors a small production server.

    python3 demo/make_demo.py /tmp/ngx-demo
    NGX_ROOT=/tmp/ngx-demo ngx ls

Certificates are generated with openssl relative to *now*, so the demo always
shows one expired cert, one expiring soon, etc. Upstreams listed in
`probe/down` are reported as refusing connections.
"""
import datetime as dt
import json
import os
import shutil
import subprocess
import sys

root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "demo-root")
shutil.rmtree(root, ignore_errors=True)
now = dt.datetime.now(dt.timezone.utc)


def p(*parts):
    path = os.path.join(root, *[x.lstrip("/") for x in parts])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    return path


def write(path, text):
    with open(p(path), "w") as f:
        f.write(text)


NGINX_CONF = """user www-data;
worker_processes auto;
pid /run/nginx.pid;

events { worker_connections 768; }

http {
    include mime.types;
    default_type application/octet-stream;
    access_log /var/log/nginx/access.log;
    error_log /var/log/nginx/error.log;
    include /etc/nginx/conf.d/*.conf;
    include /etc/nginx/sites-enabled/*;
}
"""
write("/etc/nginx/nginx.conf", NGINX_CONF)
os.makedirs(p("/etc/nginx/conf.d/x"), exist_ok=True)
os.rmdir(p("/etc/nginx/conf.d/x"))

LE = "/etc/letsencrypt/live"


def ssl_block(name):
    return f"""    ssl_certificate {LE}/{name}/fullchain.pem;
    ssl_certificate_key {LE}/{name}/privkey.pem;
"""


def redirect(names):
    return f"""server {{
    listen 80;
    listen [::]:80;
    server_name {names};
    return 301 https://$host$request_uri;
}}

"""


sites = {
    "example.conf": redirect("example.com www.example.com") + f"""server {{
    listen 443 ssl;
    listen [::]:443 ssl;
    server_name example.com www.example.com;
{ssl_block("example.com")}
    location / {{
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
    }}
}}
""",
    "api.conf": redirect("api.example.com") + f"""server {{
    listen 443 ssl;
    server_name api.example.com;
{ssl_block("api.example.com")}
    access_log /var/log/nginx/api.access.log ngx;
    location / {{
        proxy_pass http://127.0.0.1:8080;
    }}
}}
""",
    "blog.conf": redirect("blog.example.com") + f"""server {{
    listen 443 ssl;
    server_name blog.example.com;
{ssl_block("blog.example.com")}
    root /var/www/blog;
    index index.php;

    location / {{
        try_files $uri $uri/ /index.php;
    }}
    location ~ \\.php$ {{
        include fastcgi_params;
        fastcgi_pass unix:/run/php/php8.3-fpm.sock;
    }}
}}
""",
    "staging.conf": f"""upstream staging_app {{
    server 127.0.0.1:4000;
}}

""" + redirect("staging.example.com www.staging.example.com") + f"""server {{
    listen 443 ssl;
    server_name staging.example.com www.staging.example.com;
{ssl_block("staging.example.com")}
    access_log /var/log/nginx/staging.access.log ngx;
    error_log  /var/log/nginx/staging.error.log;

    location / {{
        proxy_pass http://staging_app;
    }}
    location /static/ {{
        alias /var/www/staging/public;
    }}
    location /ws {{
        proxy_pass http://127.0.0.1:4000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }}
}}
""",
    "status.conf": """server {
    listen 80;
    server_name status.example.com;
    location / {
        proxy_pass http://127.0.0.1:3001;
    }
}
""",
    "old.conf": redirect("old.example.com") + f"""server {{
    listen 443 ssl;
    # legacy site, kept for redirects
    server_name old.example.com www.example.com;
{ssl_block("old.example.com")}
    location / {{
        proxy_pass http://127.0.0.1:5000;
    }}
}}
""",
    "wiki.conf": """server {
    listen 80;
    server_name wiki.example.com;
    root /var/www/wiki;
}
""",
}
for name, text in sites.items():
    write(f"/etc/nginx/sites-available/{name}", text)
    if name != "wiki.conf":
        os.symlink(f"/etc/nginx/sites-available/{name}", p(f"/etc/nginx/sites-enabled/{name}"))
os.makedirs(p("/var/www/wiki"), exist_ok=True)

# ── certificates ──
ca_dir = p("/tmp/ca/x")[:-2]
subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                "-keyout", f"{ca_dir}/ca.key", "-out", f"{ca_dir}/ca.pem", "-days", "3650",
                "-subj", "/C=US/O=Let's Encrypt/CN=R11"], check=True, capture_output=True)


def cert(name, sans, days_from_now):
    d = p(f"{LE}/{name}/x")[:-2]
    not_after = (now + dt.timedelta(days=days_from_now)).strftime("%Y%m%d%H%M%SZ")
    not_before = (now - dt.timedelta(days=200)).strftime("%Y%m%d%H%M%SZ")
    subprocess.run(["openssl", "req", "-new", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                    "-nodes", "-keyout", f"{d}/privkey.pem", "-out", f"{d}/fullchain.pem",
                    "-CA", f"{ca_dir}/ca.pem", "-CAkey", f"{ca_dir}/ca.key",
                    "-subj", f"/CN={sans[0]}", "-addext", "subjectAltName=" + ",".join(f"DNS:{s}" for s in sans),
                    "-not_before", not_before, "-not_after", not_after], check=True, capture_output=True)


def renewal(name, auto=True):
    extra = "" if auto else "autorenew = False\n"
    write(f"/etc/letsencrypt/renewal/{name}.conf", f"""version = 2.11.0
archive_dir = /etc/letsencrypt/archive/{name}
cert = {LE}/{name}/cert.pem
fullchain = {LE}/{name}/fullchain.pem

[renewalparams]
account = 0123456789abcdef
authenticator = nginx
{extra}server = https://acme-v02.api.letsencrypt.org/directory
""")


cert("example.com", ["example.com", "www.example.com"], 75); renewal("example.com")
cert("api.example.com", ["api.example.com"], 9); renewal("api.example.com", auto=False)
cert("blog.example.com", ["blog.example.com"], 142); renewal("blog.example.com")
cert("staging.example.com", ["staging.example.com", "www.staging.example.com"], -29); renewal("staging.example.com", auto=False)
cert("old.example.com", ["old.example.com"], 240); renewal("old.example.com")
shutil.rmtree(p("/tmp"))
write("/etc/cron.d/certbot", "0 */12 * * * root certbot -q renew\n")
os.makedirs(p("/etc/letsencrypt/accounts/acme-v02.api.letsencrypt.org"), exist_ok=True)

# ── probes & logs ──
write("/probe/down", "127.0.0.1:4000\n")


def ts_err(t):
    return t.astimezone().strftime("%Y/%m/%d %H:%M:%S")


def ts_acc(t):
    return t.astimezone().strftime("%d/%b/%Y:%H:%M:%S %z")


t1, t2, t3 = now - dt.timedelta(minutes=12), now - dt.timedelta(minutes=14), now - dt.timedelta(hours=1, minutes=36)
req = 'client: 203.0.113.7, server: staging.example.com, request: "GET / HTTP/1.1", upstream: "http://127.0.0.1:4000/", host: "staging.example.com"'
write("/var/log/nginx/staging.error.log", "\n".join([
    f"{ts_err(t3)} [error] 811#811: *1201 upstream prematurely closed connection while reading response header from upstream, {req}",
    f"{ts_err(t2)} [error] 811#811: *1530 connect() failed (111: Connection refused) while connecting to upstream, {req}",
    f"{ts_err(t1)} [error] 811#811: *1532 connect() failed (111: Connection refused) while connecting to upstream, {req}",
]) + "\n")

lines = []
for i in range(1284):
    t = now - dt.timedelta(seconds=i * 60)
    status = 502 if i % 1000 < 787 and i < 787 else 200
    lines.append(f'203.0.113.{i % 250} - - [{ts_acc(t)}] "GET /p/{i} HTTP/1.1" {status} 157 "-" "Mozilla/5.0" rt=0.{(i * 37) % 900 + 50:03d}')
lines.reverse()
write("/var/log/nginx/staging.access.log", "\n".join(lines) + "\n")
api = [f'198.51.100.{i % 200} - - [{ts_acc(now - dt.timedelta(seconds=i * 9))}] "GET /v1/items HTTP/1.1" 200 812 "-" "app/1.0" rt=0.0{i % 9 + 1}0'
       for i in range(9312)]
api.reverse()
write("/var/log/nginx/api.access.log", "\n".join(api) + "\n")

# ── stable IDs matching the order sites were created ──
order = ["example.conf#example.com", "api.conf#api.example.com", "blog.conf#blog.example.com",
         "staging.conf#staging.example.com", "status.conf#status.example.com", "old.conf#old.example.com",
         "wiki.conf#wiki.example.com"]
write("/var/lib/ngx/state.json", json.dumps({"next_id": len(order) + 1, "ids": {k: i + 1 for i, k in enumerate(order)}}, indent=2))
print(f"demo sandbox created in {root}\n  NGX_ROOT={root} ngx ls")
