//! The fallback ladder over [`VoiceSource`] (`docs/design.md#voice-tiers`).
//! `VoiceSource` lives in `teleprompt-core` because `teleprompt-schedule`
//! names it too and must not depend on this crate.

pub use teleprompt_core::voice::{VoiceSource, VoiceTier};

/// The reason kept is the *first* rejection, so it explains why the tier
/// the author asked for was not used.
pub fn resolve_source(
    requested: VoiceSource,
    available: &dyn Fn(VoiceSource) -> Result<(), String>,
) -> Result<VoiceTier, String> {
    let mut tier = requested;
    let mut reason: Option<String> = None;

    loop {
        match available(tier) {
            // A rejection is what moved the ladder down, so a lower tier
            // always has its reason.
            Ok(()) => {
                return match reason {
                    None => Ok(VoiceTier::delivered(requested)),
                    Some(why) => VoiceTier::downgraded(requested, tier, why),
                };
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
