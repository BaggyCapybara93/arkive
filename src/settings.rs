
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub enable_trash: bool,
    pub verbose: bool,
    pub recursive: bool,
    pub dry_run: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enable_trash: true,
            verbose: false,
            recursive: false,
            dry_run: false,
        }
    }
}   


