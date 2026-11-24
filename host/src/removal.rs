//! The platform administrator's deletion of an account or a company for good (docs/api.md, 「The console face」):
//! the registry of the account's personal tools, its copies of every company's apps and its seen marks, or a
//! company's catalogue, counted and taken.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Value, json};

use crate::catalogue::{Catalogue, catalogues, uninstall};
use crate::registry::{Registry, owner_name};

// An account's, a unit's or a company's id: positive decimal digits.
pub fn id(given: &str) -> Option<&str> {
  let digits = !given.is_empty() && given.len() <= 20 && given.bytes().all(|byte| byte.is_ascii_digit());
  (digits && given.bytes().any(|byte| byte != b'0')).then_some(given)
}

// Every company's catalogue standing under the data root.
fn every_catalogue(data_root: &Path) -> Vec<Catalogue> {
  let mut companies: Vec<String> = fs::read_dir(catalogues(data_root))
    .into_iter()
    .flatten()
    .flatten()
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .filter_map(|entry| entry.file_name().into_string().ok())
    .filter(|name| owner_name(name))
    .collect();
  companies.sort();
  companies.iter().filter_map(|company| Catalogue::of(data_root, company).ok()).collect()
}

/// What the host keeps of an account, and the removal takes.
pub fn account_held(data_root: &Path, account: &str) -> Value {
  let owned = Registry::of(data_root).owned(account);
  let catalogues = every_catalogue(data_root);
  let installs: usize = catalogues
    .iter()
    .map(|catalogue| catalogue.list().iter().filter(|record| record.installs.contains_key(account)).count())
    .sum();
  let seen = catalogues.iter().filter(|catalogue| catalogue.seen_at(account).is_some()).count();
  json!({ "personal-tools": owned.entries().0.len(), "installs": installs, "seen-marks": seen })
}

/// Everything the host keeps of an account taken: its registry's pointers unpointed under its lock and its folder
/// then taken whole, its copy of every app uninstalled and its seen marks forgotten. Answers what it took.
pub fn remove_account(data_root: &Path, account: &str) -> Result<Value, String> {
  let held = account_held(data_root, account);
  let registry = Registry::of(data_root);
  let owned = registry.owned(account);
  let folder = registry.owners_root().join(account);

  if folder.is_dir() {
    let _locked = owned.lock().map_err(|why| format!("the owner's registry could not be locked: {why}"))?;
    for pointer in owned.entries().0 {
      owned
        .unpoint(&pointer.name)
        .map_err(|why| format!("{} could not be unpointed: {why}", pointer.name))?;
    }
  }
  gone(fs::remove_dir_all(&folder))
    .map_err(|why| format!("the owner's registry could not be taken: {why}"))?;

  for catalogue in every_catalogue(data_root) {
    for record in catalogue.list() {
      if record.installs.contains_key(account) {
        uninstall(&catalogue, &record.id, account)?;
      }
    }
    catalogue.forget_seen(account)?;
  }

  Ok(held)
}

/// What the host keeps of a company, by its catalogue; nothing of a unit below a company.
pub fn unit_held(data_root: &Path, unit: &str, company: Option<&str>) -> Result<Value, String> {
  let apps = match company {
    Some(company) if company == unit => Catalogue::of(data_root, company)?.list().len(),
    _ => 0,
  };
  Ok(json!({ "catalogue-apps": apps }))
}

/// A company's catalogue folder taken whole; nothing for a unit below a company. Answers what it took.
pub fn remove_unit(data_root: &Path, unit: &str, company: Option<&str>) -> Result<Value, String> {
  let held = unit_held(data_root, unit, company)?;
  if company == Some(unit) {
    gone(fs::remove_dir_all(catalogues(data_root).join(unit)))
      .map_err(|why| format!("the company's catalogue could not be taken: {why}"))?;
  }
  Ok(held)
}

// A removal of something already gone is done.
fn gone(removed: io::Result<()>) -> io::Result<()> {
  match removed {
    Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(()),
    other => other,
  }
}
