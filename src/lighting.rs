//! Light on the map while the game runs: the map's `[Lighting]`, light
//! sources (lamp posts and other lit buildings, radiation), and the
//! lightning storm's darkening. The pictures are drawn at full light and each
//! sprite is tinted by the light of its cell.

use std::collections::HashMap;

use bevy::prelude::*;

/// Brightest a cell can be, in thousandths.
pub const MAX_LIGHT: i32 = 2000;

/// One set of `[Lighting]` values: ambient and tint in hundredths, ground and
/// level in thousandths, each truncated as the engine stores them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightSet {
    pub ambient: i32,
    pub tint: [i32; 3],
    pub ground: i32,
    /// Added per height level.
    pub level: i32,
}

/// A light: where its middle is in leptons, how far it reaches, and what it
/// adds at the middle to the brightness and to each channel of the tint (in
/// thousandths), falling evenly to nothing at its edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub x: i32,
    pub y: i32,
    pub radius: i32,
    pub intensity: i32,
    pub tint: [i32; 3],
}

/// A type's `LightVisibility`, `LightIntensity` and `Light…Tint`, in leptons
/// and thousandths.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TypeLight {
    pub visibility: i32,
    pub intensity: i32,
    pub tint: [i32; 3],
}

impl TypeLight {
    /// Reads a type's light from its rules section; `None` without one.
    // Each value is stored as the fraction times 1000 plus 0.1, truncated.
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let number = |key: &str, default: f64| get(key).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(default);
        let stored = |value: f64| (value * 1000.0 + 0.1) as i32;
        let intensity = stored(number("LightIntensity", 0.0));
        (intensity != 0).then(|| Self {
            // DATA: the defaults rules.ini's comments give.
            visibility: number("LightVisibility", 5000.0) as i32,
            intensity,
            tint: [stored(number("LightRedTint", 1.0)), stored(number("LightGreenTint", 1.0)), stored(number("LightBlueTint", 1.0))],
        })
    }

    pub fn at(&self, x: i32, y: i32) -> Source {
        Source { x, y, radius: self.visibility, intensity: self.intensity, tint: self.tint }
    }
}

/// Everything about light that is fixed for a match.
#[derive(Clone, Default)]
pub struct LightSetup {
    pub normal: LightSet,
    /// The `Ion…` set, used while a lightning storm rages.
    pub ion: LightSet,
    /// `ExtraUnitLight`, `ExtraInfantryLight`, `ExtraAircraftLight`, in
    /// thousandths.
    pub extra_unit: i32,
    pub extra_infantry: i32,
    pub extra_aircraft: i32,
    /// `AmbientChangeRate` in frames and `AmbientChangeStep` in hundredths:
    /// how the ambient light goes over to a new value.
    pub change_frames: u64,
    pub change_step: i32,
    /// Lights that never change. Terrain objects make none, although some
    /// types (the ore drills) give a `LightIntensity`.
    pub statics: Vec<Source>,
    /// Each building type's light and art.ini `ExtraLight`, by type index.
    pub buildings: Vec<(Option<TypeLight>, i32)>,
    /// `[Radiation] RadLightFactor`, and the tint of a fresh site (`RadColor`
    /// times `RadTintFactor`, in thousandths).
    pub rad_light_factor: f64,
    pub rad_tint: [i32; 3],
}

impl LightSetup {
    /// Reads the `[Lighting]` of a map and the rules' light settings.
    // The `Ion…` keys default to the plain ones.
    pub fn read(map: &ra2_formats::ini::Ini, rules: Option<&ra2_formats::ini::Ini>) -> Self {
        let value = |key: &str| map.get("Lighting", key).and_then(|text| text.split(';').next()?.trim().parse::<f64>().ok());
        let hundredths = |v: f64| (v * 100.0) as i32;
        let thousandths = |v: f64| (v * 1000.0) as i32;
        let set = |prefix: &str, base: Option<LightSet>| {
            let key = |name: &str| value(&format!("{prefix}{name}"));
            LightSet {
                ambient: key("Ambient").map(hundredths).or(base.map(|b| b.ambient)).unwrap_or(100),
                tint: [
                    key("Red").map(hundredths).or(base.map(|b| b.tint[0])).unwrap_or(100),
                    key("Green").map(hundredths).or(base.map(|b| b.tint[1])).unwrap_or(100),
                    key("Blue").map(hundredths).or(base.map(|b| b.tint[2])).unwrap_or(100),
                ],
                ground: key("Ground").map(thousandths).or(base.map(|b| b.ground)).unwrap_or(0),
                level: key("Level").map(thousandths).or(base.map(|b| b.level)).unwrap_or(0),
            }
        };
        let normal = set("", None);
        let ion = set("Ion", Some(normal));
        let rule = |section: &str, key: &str| rules.and_then(|rules| rules.get(section, key)).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok());
        let extra = |key: &str| (rule("AudioVisual", key).unwrap_or(0.0) * 1000.0) as i32;
        let colour: Vec<f64> = rules
            .and_then(|rules| rules.get("Radiation", "RadColor"))
            .map(|v| v.split(';').next().unwrap_or("").split(',').filter_map(|n| n.trim().parse().ok()).collect())
            .unwrap_or_default();
        let tint_factor = rule("Radiation", "RadTintFactor").unwrap_or(1.0);
        let rad_tint = std::array::from_fn(|i| ((colour.get(i).copied().unwrap_or(if i == 1 { 255.0 } else { 0.0 }) * 1000.0 / 255.0 * tint_factor).min(MAX_LIGHT as f64)) as i32);
        Self {
            normal,
            ion,
            extra_unit: extra("ExtraUnitLight"),
            extra_infantry: extra("ExtraInfantryLight"),
            extra_aircraft: extra("ExtraAircraftLight"),
            change_frames: (rule("AudioVisual", "AmbientChangeRate").unwrap_or(0.0) * 900.0) as u64,
            change_step: (rule("AudioVisual", "AmbientChangeStep").unwrap_or(0.0) * 100.0) as i32,
            statics: Vec::new(),
            buildings: Vec::new(),
            rad_light_factor: rule("Radiation", "RadLightFactor").unwrap_or(0.1),
            rad_tint,
        }
    }
}

/// How a picture takes its cell's light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightRule {
    /// Through the cell's own tinted palette, as the ground: tiles, smudges,
    /// rocks, trees, bridges (`raise` levels up).
    Ground { raise: i32 },
    /// Through a shared palette at the cell's brightness with no tint, plus a
    /// fixed amount: buildings, vehicles, soldiers.
    Object { extra: i32 },
}

/// A sprite lit by a cell.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Lit {
    pub cell: (i32, i32),
    pub level: i32,
    pub rule: LightRule,
}

/// Whether a lightning storm rages, for the darkening. Whatever runs the storm
/// sets it.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct StormLight(pub bool);

/// The light on the map now.
#[derive(Resource)]
pub struct WorldLight {
    pub setup: LightSetup,
    /// The ambient light now and the value it is going over to, in hundredths.
    ambient: i32,
    target: i32,
    next_step: u64,
    storm: bool,
    sources: Vec<Source>,
    /// Per cell, what the sources add: brightness and each tint channel.
    sums: HashMap<(i32, i32), (i32, [i32; 3])>,
    /// Counts every change, so that pictures are tinted again only then.
    pub generation: u64,
}

impl WorldLight {
    pub fn new(setup: LightSetup) -> Self {
        let ambient = setup.normal.ambient;
        let mut light = Self { setup, ambient, target: ambient, next_step: 0, storm: false, sources: Vec::new(), sums: HashMap::new(), generation: 1 };
        let statics = light.setup.statics.clone();
        light.set_sources(statics);
        light
    }

    /// Starts or ends the storm's light: the `Ion…` ground, level and tint at
    /// once, the ambient light stepping over to `IonAmbient` (and back after).
    pub fn set_storm(&mut self, storm: bool, tick: u64) {
        if storm == self.storm {
            return;
        }
        self.storm = storm;
        self.target = if storm { self.setup.ion.ambient } else { self.setup.normal.ambient };
        self.next_step = tick;
        self.generation += 1;
    }

    /// Moves the ambient light one step towards its target when the step is
    /// due.
    // Every `AmbientChangeRate` minutes by `AmbientChangeStep`; at a rate of
    // nought it never changes.
    pub fn step(&mut self, tick: u64) {
        if self.ambient == self.target || self.setup.change_frames == 0 || tick < self.next_step {
            return;
        }
        self.next_step = tick + self.setup.change_frames;
        let step = self.setup.change_step.max(1);
        self.ambient = if self.ambient < self.target { (self.ambient + step).min(self.target) } else { (self.ambient - step).max(self.target) };
        self.generation += 1;
    }

    /// Replaces the lights on the map; the cells' sums are worked out again
    /// only when they differ.
    pub fn set_sources(&mut self, mut sources: Vec<Source>) {
        sources.sort_by_key(|s| (s.x, s.y, s.radius, s.intensity, s.tint));
        if sources == self.sources && self.generation > 1 {
            return;
        }
        self.sums.clear();
        for source in &sources {
            let r = source.radius.max(1);
            let reach = r / 256 + 1;
            let (cx, cy) = (source.x.div_euclid(256), source.y.div_euclid(256));
            for y in cy - reach..=cy + reach {
                for x in cx - reach..=cx + reach {
                    // The distance from the cell's middle, truncated; the
                    // share is in thousandths.
                    let (dx, dy) = ((x * 256 + 128 - source.x) as f64, (y * 256 + 128 - source.y) as f64);
                    let d = dx.hypot(dy) as i32;
                    if d > r {
                        continue;
                    }
                    let share = (r * 1000 - d * 1000) / r;
                    let sum = self.sums.entry((x, y)).or_insert((0, [0; 3]));
                    sum.0 += source.intensity * share / 1000;
                    for c in 0..3 {
                        sum.1[c] += source.tint[c] * share / 1000;
                    }
                }
            }
        }
        self.sources = sources;
        self.generation += 1;
    }

    /// The multiplier for each colour channel (1.0 leaves the art as drawn)
    /// of a picture on a cell at a height level.
    // A cell's brightness is the ambient light plus its sources, plus `Level`
    // per height level, less `Ground`, held to 0..2000. Its tint, the map's
    // plus the sources', is normalised so that the strongest channel is 1000.
    // The ground is drawn at that brightness through the cell's palette, which
    // carries the normalised tint; objects and the raised bridge level at the
    // brightness times the strongest channel, objects through a shared
    // untinted palette. Brightness picks a shade row (127 per 100%).
    pub fn factor(&self, cell: (i32, i32), level: i32, rule: LightRule) -> [f32; 3] {
        let set = if self.storm { &self.setup.ion } else { &self.setup.normal };
        let (added, added_tint) = self.sums.get(&cell).copied().unwrap_or((0, [0; 3]));
        let raise = match rule {
            LightRule::Ground { raise } => raise,
            LightRule::Object { .. } => 0,
        };
        let brightness = (self.ambient * 10 + added + set.level * (level + raise) - set.ground).clamp(0, MAX_LIGHT);
        let tint: [i32; 3] = std::array::from_fn(|c| (self.setup.normal.tint[c] * 10 + added_tint[c]).clamp(0, MAX_LIGHT));
        let strongest = tint.into_iter().max().unwrap_or(1000).max(1);
        let scaled = (brightness * strongest / 1000).min(MAX_LIGHT);
        let light = match rule {
            LightRule::Ground { raise: 0 } => shade_step(brightness),
            LightRule::Ground { .. } => shade_step(scaled),
            LightRule::Object { extra } => shade_step(scaled) + extra,
        };
        // While the storm rages every palette takes the `Ion` tint in place
        // of its own.
        let channels: [i32; 3] = match (rule, self.storm) {
            (_, true) => set.tint.map(|c| light * (c * 10).clamp(0, MAX_LIGHT) / 1000),
            (LightRule::Ground { .. }, false) => tint.map(|c| light * c / strongest),
            (LightRule::Object { .. }, false) => [light; 3],
        };
        channels.map(|c| c.max(0) as f32 / 1000.0)
    }

    /// The sprite colour for a picture with this light and see-through-ness.
    pub fn colour(&self, lit: &Lit, alpha: f32) -> Color {
        tint_colour(self.factor(lit.cell, lit.level, lit.rule), alpha)
    }
}

/// A sprite colour that multiplies the art's palette colours by `factor`.
// Sprites multiply in linear light; the engine multiplies the palette's
// gamma-encoded values, so the factor is raised to the display's gamma.
pub fn tint_colour(factor: [f32; 3], alpha: f32) -> Color {
    let [r, g, b] = factor.map(|f| f.max(0.0).powf(2.2));
    Color::LinearRgba(LinearRgba::new(r, g, b, alpha))
}

/// Rounds a brightness to one of the engine's shade steps and returns it in
/// thousandths.
// Brightness becomes a shade row, `brightness * 261 >> 11` capped at 254,
// so 1000 is row 127.
pub fn shade_step(brightness: i32) -> i32 {
    let row = ((brightness.max(0) * 261) >> 11).min(254);
    row * 1000 / 127
}

/// Tints the sprites that carry a [`Lit`], when the light or their place has
/// changed.
pub fn apply(light: Option<Res<WorldLight>>, mut seen: Local<u64>, mut sprites: Query<(Ref<Lit>, &mut Sprite)>) {
    let Some(light) = light else {
        return;
    };
    let all = *seen != light.generation;
    *seen = light.generation;
    for (lit, mut sprite) in &mut sprites {
        if !all && !lit.is_changed() {
            continue;
        }
        let wanted = light.colour(&lit, sprite.color.alpha());
        if sprite.color != wanted {
            sprite.color = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> LightSetup {
        let normal = LightSet { ambient: 85, tint: [100, 100, 100], ground: 0, level: 32 };
        LightSetup { normal, ion: LightSet { ambient: 50, tint: [60, 60, 100], ..normal }, change_frames: 180, change_step: 20, ..default() }
    }

    #[test]
    fn a_lamp_lights_its_cell_and_fades_to_its_edge() {
        let mut light = WorldLight::new(setup());
        let ground = LightRule::Ground { raise: 0 };
        let plain = light.factor((10, 10), 0, ground)[0];
        light.set_sources(vec![Source { x: 10 * 256 + 128, y: 10 * 256 + 128, radius: 1024, intensity: 200, tint: [50, 50, 10] }]);
        let middle = light.factor((10, 10), 0, ground)[0];
        let near = light.factor((12, 10), 0, ground)[0];
        assert!(middle > near && near > plain, "{middle} {near} {plain}");
        assert_eq!(light.factor((15, 10), 0, ground)[0], plain);
    }

    #[test]
    fn the_storm_darkens_by_steps() {
        let mut light = WorldLight::new(setup());
        let object = LightRule::Object { extra: 0 };
        let before = light.factor((0, 0), 0, object)[0];
        light.set_storm(true, 100);
        light.step(100);
        let first = light.factor((0, 0), 0, object)[0];
        light.step(150);
        assert_eq!(light.factor((0, 0), 0, object)[0], first, "stepped before the rate was up");
        light.step(280);
        let second = light.factor((0, 0), 0, object)[0];
        assert!(before > first && first > second, "{before} {first} {second}");
        for tick in (460..2000).step_by(180) {
            light.step(tick);
        }
        assert_eq!(light.ambient, 50);
        light.set_storm(false, 3000);
        for tick in (3000..5000).step_by(180) {
            light.step(tick);
        }
        assert_eq!(light.factor((0, 0), 0, object)[0], before);
    }
}
