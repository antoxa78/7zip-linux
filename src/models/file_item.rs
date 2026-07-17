use std::cell::RefCell;

use glib::prelude::*;
use glib::subclass::prelude::*;
use glib::{ParamSpec, ParamSpecBoolean, ParamSpecString, ParamSpecUInt64, Value};

#[derive(Debug, Default)]
pub struct FileItemInner {
    pub name: RefCell<String>,
    pub path: RefCell<String>,
    pub is_dir: RefCell<bool>,
    pub size: RefCell<u64>,
    pub modified: RefCell<u64>,
    pub created: RefCell<u64>,
    pub accessed: RefCell<u64>,
    pub file_type: RefCell<String>,
}

#[glib::object_subclass]
impl ObjectSubclass for FileItemInner {
    const NAME: &'static str = "FileItem";
    type Type = super::FileItem;
    type Interfaces = ();
}

impl ObjectImpl for FileItemInner {
    fn properties() -> &'static [ParamSpec] {
        use std::sync::LazyLock;
        static PROPERTIES: LazyLock<Vec<ParamSpec>> = LazyLock::new(|| {
            vec![
                ParamSpecString::builder("name").build(),
                ParamSpecString::builder("path").build(),
                ParamSpecString::builder("file-type").build(),
                ParamSpecUInt64::builder("size").build(),
                ParamSpecUInt64::builder("modified").build(),
                ParamSpecUInt64::builder("created").build(),
                ParamSpecUInt64::builder("accessed").build(),
                ParamSpecBoolean::builder("is-dir").build(),
            ]
        });
        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
        match pspec.name() {
            "name" => {
                *self.name.borrow_mut() = value.get().unwrap();
            }
            "path" => {
                *self.path.borrow_mut() = value.get().unwrap();
            }
            "file-type" => {
                *self.file_type.borrow_mut() = value.get().unwrap();
            }
            "size" => {
                *self.size.borrow_mut() = value.get().unwrap();
            }
            "modified" => {
                *self.modified.borrow_mut() = value.get().unwrap();
            }
            "created" => {
                *self.created.borrow_mut() = value.get().unwrap();
            }
            "accessed" => {
                *self.accessed.borrow_mut() = value.get().unwrap();
            }
            "is-dir" => {
                *self.is_dir.borrow_mut() = value.get().unwrap();
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
        match pspec.name() {
            "name" => self.name.borrow().to_value(),
            "path" => self.path.borrow().to_value(),
            "file-type" => self.file_type.borrow().to_value(),
            "size" => self.size.borrow().to_value(),
            "modified" => self.modified.borrow().to_value(),
            "created" => self.created.borrow().to_value(),
            "accessed" => self.accessed.borrow().to_value(),
            "is-dir" => self.is_dir.borrow().to_value(),
            _ => unimplemented!(),
        }
    }
}

glib::wrapper! {
    pub struct FileItem(ObjectSubclass<FileItemInner>);
}

impl FileItem {
    pub fn new(
        name: &str,
        path: &str,
        is_dir: bool,
        size: u64,
        modified: u64,
        created: u64,
        accessed: u64,
        file_type: &str,
    ) -> Self {
        glib::Object::builder()
            .property("name", name)
            .property("path", path)
            .property("is-dir", is_dir)
            .property("size", size)
            .property("modified", modified)
            .property("created", created)
            .property("accessed", accessed)
            .property("file-type", file_type)
            .build()
    }

    pub fn name(&self) -> String {
        self.property::<String>("name")
    }

    pub fn path(&self) -> String {
        self.property::<String>("path")
    }

    pub fn is_dir(&self) -> bool {
        self.property::<bool>("is-dir")
    }

    pub fn size(&self) -> u64 {
        self.property::<u64>("size")
    }

    pub fn modified(&self) -> u64 {
        self.property::<u64>("modified")
    }

    pub fn created(&self) -> u64 {
        self.property::<u64>("created")
    }

    pub fn accessed(&self) -> u64 {
        self.property::<u64>("accessed")
    }

    pub fn file_type(&self) -> String {
        self.property::<String>("file-type")
    }

    pub fn size_display(&self) -> String {
        crate::utils::format::format_size(self.size())
    }

    pub fn modified_display(&self) -> String {
        crate::utils::format::format_timestamp(self.modified())
    }

    pub fn created_display(&self) -> String {
        crate::utils::format::format_timestamp(self.created())
    }

    pub fn accessed_display(&self) -> String {
        crate::utils::format::format_timestamp(self.accessed())
    }
}
