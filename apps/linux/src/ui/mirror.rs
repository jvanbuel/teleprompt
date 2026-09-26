//! A container that can show its child mirrored, for a beam-splitter
//! prompter. The transform is GTK's, so clicks land where they appear.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Default)]
    pub struct Mirror {
        pub mirrored: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Mirror {
        const NAME: &'static str = "TelepromptMirror";
        type Type = super::Mirror;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Mirror {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Mirror {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match self.obj().first_child() {
                Some(child) => child.measure(orientation, for_size),
                None => (0, 0, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            let transform = self.mirrored.get().then(|| {
                gtk::gsk::Transform::new()
                    .translate(&gtk::graphene::Point::new(width as f32, 0.0))
                    .scale(-1.0, 1.0)
            });
            child.allocate(width, height, baseline, transform);
        }
    }
}

glib::wrapper! {
    pub struct Mirror(ObjectSubclass<imp::Mirror>) @extends gtk::Widget;
}

impl Mirror {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let mirror: Self = glib::Object::new();
        child.set_parent(&mirror);
        mirror.set_hexpand(true);
        mirror.set_vexpand(true);
        mirror
    }

    pub fn is_mirrored(&self) -> bool {
        self.imp().mirrored.get()
    }

    pub fn set_mirrored(&self, mirrored: bool) {
        self.imp().mirrored.set(mirrored);
        self.queue_allocate();
    }
}
