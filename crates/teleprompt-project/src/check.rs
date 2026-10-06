use teleprompt_core::Diagnostics;

use crate::project::Script;

impl Script {
    /// Compiles the script, writing nothing: its warnings, or rendered
    /// errors.
    pub fn check(&self) -> Result<Vec<String>, Diagnostics> {
        let compiled = self.compile()?;
        let mut warnings = compiled.output.warnings;
        // What is hard to say aloud, as it is said: translated, for a
        // locale with a translation.
        let display = self.path().display().to_string();
        warnings.extend(
            teleprompt_script::lint::lint(&compiled.program)
                .iter()
                .map(|d| {
                    d.render(&display)
                        .trim_start_matches("warning: ")
                        .to_string()
                }),
        );
        Ok(warnings)
    }
}
