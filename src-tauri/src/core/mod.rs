pub mod auto_backup;
pub mod autostart;
pub mod backup;
pub mod hotkey;
pub mod lightweight;
pub mod listener;
pub mod logger;
pub mod manager;

pub mod network_watch;
pub mod notify;
pub(crate) mod owner_identity;
pub mod proxy_control;
pub mod proxy_view;
pub mod runstate;
pub(crate) mod runtime_bundle;
pub mod service;
pub mod sysopt;
pub mod timer;
pub mod tray;
pub mod updater;
pub mod validate;

pub use self::{manager::CoreManager, timer::Timer, updater::SilentUpdater};
pub use notify::{handle, notification};
