#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSource {
    Recorded,
    Cloned,
    Synthetic,
}

impl VoiceSource {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "recorded" => Some(Self::Recorded),
            "cloned" => Some(Self::Cloned),
            "synthetic" => Some(Self::Synthetic),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Cloned => "cloned",
            Self::Synthetic => "synthetic",
        }
    }

    pub fn next_lower(&self) -> Option<Self> {
        match self {
            Self::Recorded => Some(Self::Cloned),
            Self::Cloned => Some(Self::Synthetic),
            Self::Synthetic => None,
        }
    }
}

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
