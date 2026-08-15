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
