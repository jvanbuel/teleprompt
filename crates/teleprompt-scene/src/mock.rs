use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash, SourceSpan};

use crate::contract::{BlockSource, Measured, SceneCompiler, Span, Validated};

pub struct MockScene;

impl SceneCompiler for MockScene {
    fn kind(&self) -> &'static str {
        "mock"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        for (i, line) in src.body.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let span = SourceSpan {
                line: src.span.line + i + 1,
                column: 1,
                len: line.len(),
            };
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("mark") => {}
                Some("wait") => match parts.next() {
                    Some(v) => {
                        if let Err(msg) = parse_duration_ms(v) {
                            diags.push(Diagnostic::error(msg).at(span));
                        }
                    }
                    None => diags.push(
                        Diagnostic::error("`wait` needs a duration")
                            .at(span)
                            .with_help("e.g. `wait 500ms`"),
                    ),
                },
                Some(other) => diags.push(
                    Diagnostic::error(format!("unknown mock directive `{other}`"))
                        .at(span)
                        .with_help("mock understands `wait <duration>` and `mark`"),
                ),
                None => {}
            }
        }

        if diags.is_empty() {
            Ok(Validated {
                scene: src.scene.clone(),
                body: src.body.clone(),
            })
        } else {
            Err(diags)
        }
    }

    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>> {
        let chunks: Vec<String> = v
            .body
            .lines()
            .collect::<Vec<_>>()
            .split(|l| l.trim() == "mark")
            .map(|lines| lines.join("\n"))
            .collect();

        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| Span {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.trim().as_bytes()),
                source,
                index,
            })
            .collect())
    }

    fn estimate(&self, span: &Span) -> Measured {
        let total: u64 = span
            .source
            .lines()
            .filter_map(|l| l.trim().strip_prefix("wait "))
            .filter_map(|v| parse_duration_ms(v.trim()).ok())
            .sum();
        Measured::Exact(total)
    }
}
