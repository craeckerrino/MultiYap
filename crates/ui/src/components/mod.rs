//! Nachbauten der shadcn/ui-Komponenten (src/components/ui) und der daraus gebauten eigenen Bausteine. Klassen,
//! data-Attribute und Aufbau wie das Original, damit Aussehen und Verhalten gleich bleiben.

pub mod badge;
pub mod button;
pub mod checkbox;
pub mod dialog;
pub mod icons;
pub mod input;
pub mod overlay;
pub mod select;
pub mod setting_row;
pub mod switch;

pub use badge::ComingSoonBadge;
pub use button::Button;
pub use input::CommitInput;
pub use select::{Select, SelectEntry};
pub use setting_row::{SettingGroup, SettingRow};
pub use switch::Switch;
