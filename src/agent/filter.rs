//! Filtering logic applied when a runtime is built.

use crate::PlatformSkill;

use super::disable::DisabledGroups;

/// Returns `true` when a platform skill must be filtered out from the runtime.
pub fn should_exclude_platform_skill(
  skill: &PlatformSkill,
  disabled: &DisabledGroups,
) -> bool {
  if disabled.all() {
    return true;
  }

  if disabled.is_empty() {
    return false;
  }

  // The actual grouping logic is intentionally kept in a centralized helper so
  // the builder API is not coupled to the concrete `PlatformSkill` definition.
  let group = platform_group(skill);
  disabled.contains(group)
}

fn platform_group(
  skill: &PlatformSkill,
) -> crate::agent::disable::DisabledGroup {
  let id = skill.id();

  match id {
    "p1" | "p2" => {
      if id == "p1" {
        crate::agent::disable::DisabledGroup::Grupo1
      } else {
        crate::agent::disable::DisabledGroup::Grupo2
      }
    }
    _ => crate::agent::disable::DisabledGroup::Grupo1,
  }
}
