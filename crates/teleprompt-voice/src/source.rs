//! The fallback ladder over [`VoiceSource`].
//!
//! `VoiceSource` itself lives in `teleprompt-core` (see
//! `teleprompt_core::voice`) because `teleprompt-schedule` names it too and
//! must not depend on this crate. It is re-exported from here so backend code
//! that already imports the ladder keeps a single import.

pub use teleprompt_core::voice::VoiceSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub requested: VoiceSource,
    pub actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

/// Walks the ladder `recorded -> cloned -> synthetic`, stopping at the first
/// tier `available` accepts. The reason recorded is the *first* rejection, not
/// the last, so the message explains why the author's own voice was not used.
pub fn resolve_source(
    requested: VoiceSource,
    available: &dyn Fn(VoiceSource) -> Result<(), String>,
) -> Result<Resolution, String> {
    let mut tier = requested;
    let mut reason: Option<String> = None;

    loop {
        match available(tier) {
            Ok(()) => {
                return Ok(Resolution {
                    requested,
                    actual: tier,
                    downgrade_reason: reason,
                });
            }
            Err(why) => {
                if reason.is_none() {
                    reason = Some(why);
                }
                match tier.next_lower() {
                    Some(next) => tier = next,
                    None => {
                        return Err(format!(
                            "no voice source available (started at `{}`): {}",
                            requested.label(),
                            reason.unwrap_or_default()
                        ))
                    }
                }
            }
        }
    }
}
