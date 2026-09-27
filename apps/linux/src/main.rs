//! `teleprompt-gtk [SCRIPT]`: the prompter as a GTK app. It launches
//! `teleprompt prompt` itself and talks to it over the prompter API.

mod ui;

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use ui::config::Config;
use ui::window::Window;

const APP_ID: &str = "io.github.jvanbuel.Teleprompt";

fn main() -> glib::ExitCode {
    ui::fonts::register();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let window: Rc<std::cell::OnceCell<Rc<Window>>> = Rc::default();
    let shown = window.clone();
    app.connect_startup(|_| {
        install_icon();
        // A prompter is a dark room: the glass is black whatever the desktop.
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("ui/style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("a display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    let window_for = move |app: &adw::Application| {
        shown
            .get_or_init(|| {
                let window = Window::new(app);
                actions(app, &window);
                window
            })
            .clone()
    };
    let activate = window_for.clone();
    app.connect_activate(move |app| activate(app).window.present());
    app.connect_open(move |app, files, _| {
        let window = window_for(app);
        window.window.present();
        if let Some(path) = files.first().and_then(|f| f.path()) {
            window.open(path);
        }
    });
    app.run()
}

type Run = Box<dyn Fn(&Rc<Window>)>;

fn actions(app: &adw::Application, window: &Rc<Window>) {
    let action = |name: &str, accels: &[&str], run: Run| {
        let weak = Rc::downgrade(window);
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| {
            if let Some(window) = weak.upgrade() {
                run(&window);
            }
        });
        app.add_action(&action);
        app.set_accels_for_action(&format!("app.{name}"), accels);
    };
    action("open", &["<Control>o"], Box::new(choose_script));
    action("session", &["<Control>n"], Box::new(choose_draft));
    action(
        "reopen",
        &[],
        Box::new(|w| {
            if let Some(last) = w.last_script() {
                w.open(last);
            }
        }),
    );
    action("take-top", &["<Control>t"], Box::new(|w| w.take(0)));
    action("undo", &["<Control>z"], Box::new(|w| w.undo()));
    action("retake", &[], Box::new(|w| w.retake()));
    action("review-said", &[], Box::new(|w| w.review_said()));
    action(
        "capture",
        &["<Control><Shift>c"],
        Box::new(|w| w.make(teleprompt_gtk::make::Job::Capture)),
    );
    action(
        "build",
        &["<Control>b"],
        Box::new(|w| w.make(teleprompt_gtk::make::Job::Build)),
    );
    action("keep", &["<Control>Return"], Box::new(|w| w.keep()));
    action(
        "screen-window",
        &["<Control><Shift>s"],
        Box::new(|w| w.screen_window()),
    );
    action("settings", &["<Control>comma"], Box::new(settings));
    let app_weak = app.downgrade();
    action(
        "quit",
        &["<Control>q"],
        Box::new(move |w| {
            w.shutdown();
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
        }),
    );
}

fn choose_script(window: &Rc<Window>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("teleprompt scripts"));
    filter.add_pattern("*.md");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Open Script")
        .filters(&filters)
        .build();
    let weak = Rc::downgrade(window);
    dialog.open(
        Some(window.file_dialog_parent()),
        gio::Cancellable::NONE,
        move |result| {
            if let (Some(window), Ok(Some(path))) = (weak.upgrade(), result.map(|f| f.path())) {
                window.open(path);
            }
        },
    );
}

/// Session mode: where the draft goes, then the terminal to record in.
fn choose_draft(window: &Rc<Window>) {
    let dialog = gtk::FileDialog::builder()
        .title("Name the New Script")
        .initial_name("session.md")
        .build();
    if let Some(dir) = window
        .last_script()
        .and_then(|s| s.parent().map(gio::File::for_path))
    {
        dialog.set_initial_folder(Some(&dir));
    }
    let weak = Rc::downgrade(window);
    dialog.save(
        Some(window.file_dialog_parent()),
        gio::Cancellable::NONE,
        move |result| {
            if let (Some(window), Ok(Some(path))) = (weak.upgrade(), result.map(|f| f.path())) {
                window.new_session(path);
            }
        },
    );
}

/// Where `teleprompt` and the speech model are, and the locale.
fn settings(window: &Rc<Window>) {
    let config = window.config();
    let group = adw::PreferencesGroup::builder()
        .description("teleprompt built with --features listen, and an unpacked sherpa-onnx streaming zipformer.")
        .build();
    let binary = path_row(
        window,
        "teleprompt binary",
        config.binary().as_deref(),
        false,
        |c, p| c.binary = Some(p),
    );
    let model = path_row(
        window,
        "Speech model",
        config.model().as_deref(),
        true,
        |c, p| c.model = Some(p),
    );
    let punctuation = path_row(
        window,
        "Punctuation model (optional)",
        config.punctuation.as_deref(),
        true,
        |c, p| c.punctuation = Some(p),
    );
    punctuation.set_tooltip_text(Some(
        "sherpa-onnx online-punct-en: gives a session's draft sentences",
    ));
    // What the app will use: the environment's, when it overrides.
    for (row, var) in [(&binary, "TELEPROMPT_BIN"), (&model, "TELEPROMPT_MODEL")] {
        if std::env::var_os(var).is_some() {
            let shown = row.subtitle().unwrap_or_default();
            row.set_subtitle(&format!("{shown} (from {var})"));
        }
    }
    let locale = adw::EntryRow::builder()
        .title("Locale")
        .text(config.locale())
        .build();
    let weak = Rc::downgrade(window);
    locale.connect_changed(move |row| {
        if let Some(window) = weak.upgrade() {
            let mut config = window.config();
            config.locale = Some(row.text().to_string());
            window.set_config(config);
        }
    });
    let countdown = adw::SwitchRow::builder()
        .title("Count down before a take")
        .subtitle("Three beats to settle before the prompter listens")
        .active(config.countdown())
        .build();
    let weak = Rc::downgrade(window);
    countdown.connect_active_notify(move |row| {
        if let Some(window) = weak.upgrade() {
            let mut config = window.config();
            config.countdown = Some(row.is_active());
            window.set_config(config);
        }
    });
    group.add(&binary);
    group.add(&model);
    group.add(&punctuation);
    group.add(&locale);
    group.add(&countdown);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    let dialog = adw::PreferencesDialog::builder().title("Settings").build();
    dialog.add(&page);
    dialog.present(Some(window.file_dialog_parent()));
}

fn path_row(
    window: &Rc<Window>,
    title: &str,
    value: Option<&std::path::Path>,
    folder: bool,
    set: fn(&mut Config, std::path::PathBuf),
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value.map_or("not set".into(), |p| p.display().to_string()))
        .build();
    let choose = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    let (weak, shown) = (Rc::downgrade(window), row.clone());
    choose.connect_clicked(move |_| {
        let Some(window) = weak.upgrade() else { return };
        let dialog = gtk::FileDialog::builder().title(shown.title()).build();
        let (weak, shown) = (Rc::downgrade(&window), shown.clone());
        let chosen = move |result: Result<gio::File, glib::Error>| {
            let (Some(window), Ok(Some(path))) = (weak.upgrade(), result.map(|f| f.path())) else {
                return;
            };
            shown.set_subtitle(&path.display().to_string());
            let mut config = window.config();
            set(&mut config, path);
            window.set_config(config);
        };
        let parent = Some(window.file_dialog_parent());
        if folder {
            dialog.select_folder(parent, gio::Cancellable::NONE, chosen);
        } else {
            dialog.open(parent, gio::Cancellable::NONE, chosen);
        }
    });
    row.add_suffix(&choose);
    row
}

/// Puts the app's icon where GTK looks for it, so the window and the
/// launcher show it without an install step.
fn install_icon() {
    let icons = glib::user_cache_dir().join("teleprompt/icons");
    let dir = icons.join("hicolor/scalable/apps");
    let svg = include_bytes!("../../icons/teleprompt.svg");
    if std::fs::create_dir_all(&dir).is_ok()
        && std::fs::write(dir.join(format!("{APP_ID}.svg")), svg).is_ok()
    {
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_search_path(&icons);
        }
        gtk::Window::set_default_icon_name(APP_ID);
    }
}
