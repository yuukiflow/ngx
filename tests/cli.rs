//! Snapshot tests of the command output (`ngx ls`, `ngx show`, `ngx check`, …)
//! against a fixed sandbox. Regenerate after an intended change with:
//!
//!     UPDATE_SNAPSHOTS=1 cargo test --test cli

use std::path::{Path, PathBuf};
use std::process::Command;

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path.trim_start_matches('/'));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    // fixed mtime: `ngx show` prints it
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_767_268_800); // 2026-01-01 12:00 UTC
    std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
}

fn link(root: &Path, target: &str, link: &str) {
    std::os::unix::fs::symlink(target, root.join(link.trim_start_matches('/'))).unwrap();
}

/// A small server: proxy, PHP, static, a redirect pair, a duplicate name,
/// a backend that's down and a disabled site. No certificates or logs, so the
/// output doesn't depend on the date.
fn sandbox() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "/etc/nginx/nginx.conf", "events {}\nhttp {\n    include /etc/nginx/sites-enabled/*;\n}\n");
    std::fs::create_dir_all(r.join("etc/nginx/sites-enabled")).unwrap();
    write(r, "/etc/nginx/sites-available/app.conf", "server {\n    listen 80;\n    server_name app.example.com www.app.example.com;\n    access_log /var/log/nginx/app.access.log;\n    location / {\n        proxy_pass http://127.0.0.1:3000;\n        proxy_set_header Upgrade $http_upgrade;\n    }\n    location /api/ {\n        proxy_pass http://127.0.0.1:8080;\n    }\n}\n");
    write(r, "/etc/nginx/sites-available/blog.conf", "server {\n    listen 80;\n    server_name blog.example.com;\n    root /var/www/blog;\n    index index.php;\n    location ~ \\.php$ {\n        include fastcgi_params;\n        fastcgi_pass unix:/run/php/php8.3-fpm.sock;\n    }\n}\n");
    write(r, "/etc/nginx/sites-available/docs.conf", "server {\n    listen 8080;\n    server_name docs.example.com;\n    root /srv/docs;\n}\n");
    write(r, "/etc/nginx/sites-available/staging.conf", "server {\n    listen 80;\n    server_name staging.example.com;\n    location / {\n        proxy_pass http://127.0.0.1:4000;\n    }\n}\n");
    write(r, "/etc/nginx/sites-available/old.conf", "server {\n    listen 80;\n    server_name old.example.com www.app.example.com;\n    location / {\n        proxy_pass http://127.0.0.1:5000;\n    }\n}\n");
    write(r, "/etc/nginx/sites-available/wiki.conf", "server {\n    listen 80;\n    server_name wiki.example.com;\n    root /var/www/wiki;\n}\n");
    for s in ["app", "blog", "docs", "staging", "old"] {
        link(r, &format!("/etc/nginx/sites-available/{s}.conf"), &format!("/etc/nginx/sites-enabled/{s}.conf"));
    }
    for d in ["var/www/blog", "srv/docs", "var/www/wiki", "run/php"] {
        std::fs::create_dir_all(r.join(d)).unwrap();
    }
    write(r, "/run/php/php8.3-fpm.sock", "");
    write(r, "/probe/down", "127.0.0.1:4000\n");
    write(
        r,
        "/var/lib/ngx/state.json",
        r#"{"next_id": 7, "ids": {"app.conf#app.example.com": 1, "blog.conf#blog.example.com": 2, "docs.conf#docs.example.com": 3, "staging.conf#staging.example.com": 4, "old.conf#old.example.com": 5, "wiki.conf#wiki.example.com": 6}}"#,
    );
    dir
}

fn ngx(root: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_ngx"))
        .args(args)
        .env("NGX_ROOT", root)
        .env("NO_COLOR", "1")
        .env("TZ", "UTC")
        .env_remove("NGX_NGINX_DIR")
        .env_remove("NGX_STATE_DIR")
        .env_remove("NGX_LETSENCRYPT_DIR")
        .output()
        .unwrap();
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (s.replace(&root.display().to_string(), "<root>"), out.status.code().unwrap_or(-1))
}

fn snapshot(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() || !path.exists() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap();
    if expected != actual {
        panic!("output of `{name}` changed (UPDATE_SNAPSHOTS=1 cargo test --test cli to accept):\n--- expected\n{expected}\n--- actual\n{actual}");
    }
}

#[test]
fn ls() {
    let sb = sandbox();
    snapshot("ls", &ngx(sb.path(), &["ls"]).0);
    snapshot("ls-all", &ngx(sb.path(), &["ls", "--all"]).0);
}

#[test]
fn show() {
    let sb = sandbox();
    snapshot("show-proxy", &ngx(sb.path(), &["show", "1"]).0);
    snapshot("show-php", &ngx(sb.path(), &["show", "blog.example.com"]).0);
    snapshot("show-down", &ngx(sb.path(), &["show", "4"]).0);
    let (out, code) = ngx(sb.path(), &["show", "nope.example.com"]);
    assert_ne!(code, 0);
    snapshot("show-unknown", &out);
}

#[test]
fn check_exit_codes() {
    let sb = sandbox();
    let (out, code) = ngx(sb.path(), &["check"]);
    assert_eq!(code, 2, "a backend is down and a name is duplicated");
    snapshot("check", &out);
}

#[test]
fn add_then_history_and_undo() {
    let sb = sandbox();
    let (_, code) = ngx(sb.path(), &["add", "-y", "-d", "new.example.com", "-l", "3005", "--ssl", "none", "--max-body", "20m", "--route", "/api=9000"]);
    assert_eq!(code, 0);
    let conf = std::fs::read_to_string(sb.path().join("etc/nginx/sites-available/new.example.com.conf")).unwrap();
    // the date in the first line changes every run
    let conf: String = conf.lines().skip(1).collect::<Vec<_>>().join("\n");
    snapshot("generated-config", &conf);
    let (hist, _) = ngx(sb.path(), &["history", "--json"]);
    assert!(hist.contains("ngx add -y -d new.example.com"));
    let (_, code) = ngx(sb.path(), &["undo", "-y"]);
    assert_eq!(code, 0);
    assert!(!sb.path().join("etc/nginx/sites-available/new.example.com.conf").exists(), "undo removes the created site");
}

#[test]
fn rejects_injection() {
    let sb = sandbox();
    for args in [
        &["add", "-y", "--ssl", "none", "-d", "x.com;include /etc/shadow", "-l", "1"][..],
        &["add", "-y", "--ssl", "none", "-d", "x.com", "-t", "static", "--root", "/var/www/x;autoindex on"],
        &["add", "-y", "--ssl", "none", "-d", "x.com", "-l", "1", "--max-body", "1m;"],
        &["add", "-y", "--ssl", "none", "-d", "x.com", "-l", "1", "--route", "/a b=1"],
    ] {
        let (out, code) = ngx(sb.path(), args);
        assert_ne!(code, 0, "{args:?} should fail: {out}");
    }
    assert!(!sb.path().join("etc/nginx/sites-available/x.com.conf").exists());
}
