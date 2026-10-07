pub mod appearance;
pub mod config;
pub mod fuzzy;
pub mod keys;
pub mod layout;
pub mod links;
pub mod prescan;
pub mod project;
pub mod ssh;
pub mod themes;

use zhell_proto::{ClientMsg, ServerMsg};

pub type Waker = Box<dyn Fn() + Send + Sync>;

pub trait SessionHost: Send {
    fn send(&self, msg: ClientMsg);

    fn try_recv(&self) -> Option<ServerMsg>;

    fn set_waker(&mut self, waker: Waker);

    fn is_alive(&self) -> bool {
        true
    }

    fn shared(&self) -> bool {
        false
    }
}
