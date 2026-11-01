//! Commands. Each submodule implements one group of `ngx` subcommands.


pub(crate) use crate::env::{self, require_root, Env};
pub(crate) use crate::nginx::site::{self, Kind, Site, Upstream, MANAGED_MARKER};
pub(crate) use crate::nginx::template::{self, Backend, Spec, Ssl};
pub(crate) use crate::nginx::{ctl, parser};
pub(crate) use crate::ops::Tx;
pub(crate) use crate::render::{self, Tone};
pub(crate) use crate::ssl;
pub(crate) use crate::status::Report;
pub(crate) use crate::validate;
pub(crate) use anyhow::{bail, Context, Result};
pub(crate) use inquire::validator::Validation;
pub(crate) use inquire::{Confirm, CustomType, Select, Text};
pub(crate) use std::path::{Path, PathBuf};


mod add;
mod auth;
mod cert;
mod check;
mod edit;
mod history;
mod layout;
mod manage;
mod misc;
mod view;

pub use add::*;
pub use auth::*;
pub use cert::*;
pub use check::*;
pub use edit::*;
pub use history::*;
pub use layout::*;
pub use manage::*;
pub use misc::*;
pub use view::*;

pub(crate) fn ok(msg: &str) {
    render::print(&[vec![render::seg("✓ ", Tone::Good), render::seg(msg, Tone::Plain)]]);
}

pub(crate) fn confirm(q: &str, default: bool, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    Ok(Confirm::new(q).with_default(default).prompt()?)
}
