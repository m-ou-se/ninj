//! Checking if a target is outdated.
//!
//! This check consists of two parts:
//!
//!  1. [`check_outputs`][outdated::check_outputs]:
//!     Checking the outputs and their logged dependencies.
//!  2. [`check_dependencies`][outdated::check_dependencies]:
//!     Checking the inputs and order-only dependencies.
//!
//! [`is_outdated`][outdated::is_outdated] performs both.

use crate::deplog::DepLog;
use crate::mtime::{StatCache, Timestamp};
use crate::spec::BuildRule;
use raw_string::unix::RawStrExt;
use raw_string::RawStr;
use std::io::{Error, ErrorKind};

/// Check if a target is outdated.
///
/// Checks all the outputs and the dependencies.
///
/// Calls `check_dep(path, is_order_only)` for every dependency. This
/// function should return true iff there's a build rule to make the
/// dependency. If there is not, and the file does not exist, an error is
/// returned.
///
/// Simply calls [`check_outputs`] followed by [`check_dependencies`].
///
/// Paths from the `dep_log` are looked up first in `stat_cache`, but never
/// stored in it. If it was not in that cache, it will be cached in
/// `dep_stat_cache` instead. (So you can modify the `dep_log` afterwards
/// by throwing out `dep_stat_cache`, but keeping `stat_cache`.)
pub fn is_outdated<'a, 'b>(
	rule: &'a BuildRule,
	dep_log: &'b DepLog,
	stat_cache: &mut StatCache<'a>,
	dep_stat_cache: &mut StatCache<'b>,
	check_dep: impl FnMut(&RawStr, bool) -> bool,
) -> Result<bool, Error> {
	let oldest_output = check_outputs(rule, dep_log, stat_cache, dep_stat_cache)?;
	check_dependencies(rule, stat_cache, oldest_output, check_dep)
}

/// Check all the outputs and their logged dependencies.
///
/// Returns [`None`] if the target is definitely out of date.
/// That happens in these cases:
///
/// - If output does not exist.
///
/// And in case the rule uses [`deps`][crate::spec::BuildCommand::deps]:
///
///  - If an output has no or outdated dependency information in the log.
///  - If an output has a logged dependency which is newer than itself.
///
/// Otherwise, it returns the [`Timestamp`] of the oldest output, for
/// comparison with the rule's [`inputs`][BuildRule::inputs].
///
/// Paths from the `dep_log` are looked up first in `stat_cache`, but never
/// stored in it. If it was not in that cache, it will be cached in
/// `dep_stat_cache` instead. (So you can modify the `dep_log` afterwards
/// by throwing out `dep_stat_cache`, but keeping `stat_cache`.)
pub fn check_outputs<'a, 'b>(
	rule: &'a BuildRule,
	dep_log: &'b DepLog,
	stat_cache: &mut StatCache<'a>,
	dep_stat_cache: &mut StatCache<'b>,
) -> Result<Option<Timestamp>, Error> {
	let mut oldest = None;

	for output in &rule.outputs {
		if let Some(mtime) = stat_cache.mtime(output.as_path())? {
			if oldest.map_or(true, |oldest| mtime < oldest) {
				oldest = Some(mtime);
			}
			if rule.command.as_ref().map_or(true, |c| !c.deps.is_some()) {
				// Don't even look up dependencies in de dependency log for
				// targets that don't use extra dependencies anyway.
				continue;
			}
			if let Some(deps) = dep_log.get(&output) {
				if deps.mtime() < Some(mtime) {
					// Our dependency information is outdated.
					return Ok(None);
				}
				for dep in deps.deps() {
					let dep_mtime = match stat_cache.cached_mtime(dep.as_path()) {
						Some(t) => t,
						None => dep_stat_cache.mtime(dep.as_path())?,
					};
					if let Some(dep_mtime) = dep_mtime {
						if mtime < dep_mtime {
							// This recorded dependency is newer than the output.
							return Ok(None);
						}
					} else {
						// This recorded dependency no longer exists.
						return Ok(None);
					}
				}
			} else {
				// Our dependency information is non-existent.
				return Ok(None);
			}
		} else {
			// This output doesn't even exist.
			return Ok(None);
		}
	}

	Ok(oldest)
}

/// Check all the input and order-only dependencies.
///
/// Returns whether the target is outdated. That is, it returns true:
///
///  - When the `oldest_output` was [`None`], or
///  - When any of the inputs does not exist or is older than the oldest output,
///    or
///  - When any of the order-only dependencies does not exist.
///
/// Needs the `oldest_output` from [`check_outputs`] to compare the
/// timestamps against. If this is [`None`], it will return `true`, but
/// still checks all the dependencies.
///
/// Calls `check_dep(path, is_order_only)` for every dependency. This
/// function should return true iff there's a build rule to make the
/// dependency. If there is not, and the file does not exist, an error is
/// returned.
pub fn check_dependencies<'a>(
	rule: &'a BuildRule,
	stat_cache: &mut StatCache<'a>,
	oldest_output: Option<Timestamp>,
	mut check_dep: impl FnMut(&RawStr, bool) -> bool,
) -> Result<bool, Error> {
	let iter_inputs = rule.inputs.iter().map(|path| (path, false));
	let iter_order_deps = rule.order_deps.iter().map(|path| (path, true));

	let mut outdated = oldest_output.is_none();

	for (path, is_order_only) in iter_inputs.chain(iter_order_deps) {
		let has_rule = check_dep(path, is_order_only);
		let mtime = stat_cache.mtime(path.as_path())?;
		if mtime.is_none() || (!is_order_only && mtime < oldest_output) {
			outdated = true;
		}
		if !has_rule && mtime.is_none() {
			return Err(Error::new(
				ErrorKind::NotFound,
				format!(
					"{:?} (needed by {:?}) not found, and there's no rule to make it.",
					path, rule.outputs[0]
				),
			));
		}
	}

	Ok(outdated)
}
