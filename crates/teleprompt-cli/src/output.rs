use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Every way a command can end. Centralised so no subcommand invents a code.
#[derive(Debug)]
pub enum Outcome {
    Ok,
    RuntimeFailure(String),
    ValidationError(Vec<String>),
    Drift,
    VoiceDowngrade,
}

pub fn exit_code_for(outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Ok => 0,
        Outcome::RuntimeFailure(_) => 1,
        Outcome::ValidationError(_) => 2,
        Outcome::Drift => 3,
        Outcome::VoiceDowngrade => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_for_matches_the_spec() {
        assert_eq!(exit_code_for(&Outcome::Ok), 0);
        assert_eq!(exit_code_for(&Outcome::RuntimeFailure("boom".into())), 1);
        assert_eq!(
            exit_code_for(&Outcome::ValidationError(vec!["bad".into()])),
            2
        );
        assert_eq!(exit_code_for(&Outcome::Drift), 3);
        assert_eq!(exit_code_for(&Outcome::VoiceDowngrade), 4);
    }
}
