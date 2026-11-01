//! `ngx ls` and `ngx show`.

#[allow(unused_imports)]
use super::*;
use crate::status::State;

pub fn ls(all: bool, json: bool) -> anyhow::Result<()> {
    let rep = Report::load(true);
    let idx: Vec<usize> = (0..rep.sites.len()).filter(|&i| all || rep.status[i].state != State::Disabled).collect();
    if json {
        let v: Vec<_> = idx.iter().map(|&i| render::json(&rep.sites[i], &rep.status[i])).collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if idx.is_empty() {
        println!("no sites found in {} — create one with `ngx add`", Env::get().nginx_dir.display());
        return Ok(());
    }
    let rows: Vec<_> = idx.iter().map(|&i| render::row(&rep.sites[i], &rep.status[i])).collect();
    let mut out = vec![vec![]];
    out.extend(render::table(&rows));
    out.push(vec![]);
    out.push(render::summary(&rep, &idx));
    let hidden = rep.sites.len() - idx.len();
    if hidden > 0 {
        out.push(vec![render::seg(format!("{hidden} disabled site{} hidden (ngx ls --all)", if hidden == 1 { "" } else { "s" }), render::Tone::Dim)]);
    }
    for g in &rep.global {
        out.push(vec![render::seg(format!("nginx: {g}"), render::Tone::Bad)]);
    }
    render::print(&out);
    Ok(())
}

pub fn show(q: &str, json: bool) -> anyhow::Result<()> {
    let rep = Report::load(true);
    let i = rep.index_of(q)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&render::json(&rep.sites[i], &rep.status[i]))?);
        return Ok(());
    }
    let mut out = vec![vec![]];
    out.extend(render::detail(&rep.sites[i], &rep.status[i]));
    render::print(&out);
    Ok(())
}
