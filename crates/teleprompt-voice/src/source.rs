//! The fallback ladder over [`VoiceSource`] (`docs/design.md#voice-tiers`).
//! `VoiceSource` lives in `teleprompt-core` because `teleprompt-schedule`
//! names it too and must not depend on this crate.

pub use teleprompt_core::voice::VoiceSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub requested: VoiceSource,
    pub actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

/// The reason kept is the *first* rejection, so it explains why the tier
/// the author asked for was not used.
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
