//! The order of a frame in a game. Orders given in `Input` reach the network
//! step the same frame; what is drawn in `View` and `Hud` is the state after it.

use bevy::prelude::*;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameSet {
    /// The mouse, keys and in-game menus: selection, orders, the camera.
    Input,
    /// A network game steps the simulation here; a local one in `FixedUpdate`.
    Net,
    /// The map and everything on it.
    View,
    /// The sidebar, bar, radar and what is laid over the map.
    Hud,
}

pub struct SchedulePlugin;

impl Plugin for SchedulePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(Update, (GameSet::Input, GameSet::Net, GameSet::View, GameSet::Hud).chain());
    }
}
