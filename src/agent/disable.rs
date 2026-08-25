//! Group-based disable rules for runtime platform skills.

use std::collections::BTreeSet;

/// Static groups that may be disabled from platform skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DisabledGroup {
  /// Disables the first static platform-skill group.
  Grupo1,
  /// Disables the second static platform-skill group.
  Grupo2,
}

/// Collection of disabled groups for a runtime build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DisabledGroups {
  groups: BTreeSet<DisabledGroup>,
  all: bool,
}

impl DisabledGroups {
  /// Starts with no disabled groups selected.
  pub const fn new() -> Self {
    Self {
      groups: BTreeSet::new(),
      all: false,
    }
  }

  /// Adds a single static group to the disabled set.
  pub fn insert(&mut self, group: DisabledGroup) {
    self.groups.insert(group);
  }

  /// Marks the entire static set as disabled.
  pub const fn disable_all(&mut self) {
    self.all = true;
  }

  /// Returns `true` when all groups are disabled.
  pub const fn all(&self) -> bool {
    self.all
  }

  /// Returns `true` when the requested group is disabled.
  pub fn contains(&self, group: DisabledGroup) -> bool {
    self.all || self.groups.contains(&group)
  }

  /// Returns `true` when no groups were selected in the flow.
  pub fn is_empty(&self) -> bool {
    !self.all && self.groups.is_empty()
  }
}
