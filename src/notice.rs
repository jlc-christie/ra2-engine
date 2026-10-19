//! A line across the top of the screen saying that something chosen isn't
//! built yet, so a button that does nothing doesn't look broken.
//!
//! Not original behaviour: every button of the original does something.

use bevy::prelude::*;


/// Shows a line for a few seconds.
#[derive(Message, Clone)]
pub struct Notice(pub String);

impl Notice {
    /// "<what> isn't available yet."
    pub fn not_built(what: &str) -> Self {
        Self(format!("{what} isn't available in this version yet."))
    }
}

/// How long a notice stays, in seconds.
const SHOWN_FOR: f64 = 3.5;

#[derive(Component)]
struct NoticeBox {
    shown_at: f64,
}

pub struct NoticePlugin;

impl Plugin for NoticePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Notice>().add_systems(Update, (show, fade).chain());
    }
}

fn show(
    mut commands: Commands,
    mut notices: MessageReader<Notice>,
    text: Res<crate::files::GameText>,
    mut images: ResMut<Assets<Image>>,
    old: Query<Entity, With<NoticeBox>>,
    time: Res<Time<Real>>,
) {
    let Some(notice) = notices.read().last().cloned() else {
        return;
    };
    info!("notice: {}", notice.0);
    let Some(font) = text.font.as_ref() else {
        return;
    };
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let (image, size) = crate::hud::text_image(font, &mut images, &notice.0);
    commands
        .spawn((
            NoticeBox { shown_at: time.elapsed_secs_f64() },
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(36.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(1000),
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            row.spawn((
                Node { padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)), border: UiRect::all(Val::Px(1.0)), ..default() },
                BackgroundColor(Color::BLACK),
                BorderColor::all(Color::srgb(0.75, 0.1, 0.08)),
                Pickable::IGNORE,
            ))
            .with_child((Node { width: Val::Px(size.x), height: Val::Px(size.y), ..default() }, ImageNode::new(image), Pickable::IGNORE));
        });
}

/// Fades a notice out over its last second, then removes it.
fn fade(mut commands: Commands, boxes: Query<(Entity, &NoticeBox)>, mut colours: Query<(&mut BackgroundColor, &ChildOf)>, mut text: Query<(&mut ImageNode, &ChildOf)>, time: Res<Time<Real>>) {
    for (entity, notice) in &boxes {
        let age = time.elapsed_secs_f64() - notice.shown_at;
        if age >= SHOWN_FOR {
            commands.entity(entity).despawn();
            continue;
        }
        let alpha = ((SHOWN_FOR - age).min(1.0)) as f32;
        for (mut colour, parent) in &mut colours {
            if parent.parent() == entity {
                colour.0 = Color::srgba(0.0, 0.0, 0.0, alpha);
            }
        }
        for (mut image, parent) in &mut text {
            if colours.get(parent.parent()).is_ok_and(|(_, grand)| grand.parent() == entity) {
                image.color = Color::srgba(1.0, 1.0, 1.0, alpha);
            }
        }
    }
}
