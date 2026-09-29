//! The models `teleprompt setup` installs are the ones the app uses when
//! Settings names none (docs/design.md#what-teleprompt-ships).

use teleprompt_gtk::models::{installed_punctuation, installed_speech, models_dir};

/// One test: it sets the process's environment.
#[test]
fn an_installed_model_is_found_where_setup_puts_it() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("TELEPROMPT_MODELS", dir.path());
    assert_eq!(models_dir(), dir.path());
    assert_eq!(installed_speech(), None);
    assert_eq!(installed_punctuation(), None);

    let speech = dir
        .path()
        .join("sherpa-onnx-streaming-zipformer-en-2023-06-26");
    let punct = dir.path().join("sherpa-onnx-online-punct-en-2024-08-06");
    std::fs::create_dir(&speech).unwrap();
    std::fs::create_dir(&punct).unwrap();
    assert_eq!(installed_speech(), Some(speech));
    assert_eq!(installed_punctuation(), Some(punct));

    std::env::remove_var("TELEPROMPT_MODELS");
    std::env::set_var("XDG_DATA_HOME", dir.path());
    assert_eq!(models_dir(), dir.path().join("teleprompt/models"));
}
