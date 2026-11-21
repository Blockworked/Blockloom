//! The runtime's sound: voices, buses and the listener.
//!
//! Blocks speak in [`Effect`]s; this module turns them into Bevy audio.
//! Every `play` spawns a voice entity carrying an [`AudioPlayer`], so rapid
//! replays overlap rather than cut each other off. A voice is either global
//! (music, UI) or positional: it follows an actor around, panned by where it
//! stands relative to the camera and quieter with distance. Bevy's spatial
//! audio is stereo panning only, so the falloff is computed here, per frame,
//! from the same distance model [`blockloom_core::sound`] describes.
//!
//! The mix has three buses - master, music and effects - seeded from the
//! saved [`SoundMixer`](blockloom_core::sound::SoundMixer) on every rebuild
//! and moved by `set bus volume` without touching the document, the same
//! split world gravity has. The effective gain of a voice is always
//! `master * bus * voice * distance`.

use bevy::audio::{
    AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, SpatialAudioSink,
    Volume,
};
use bevy::prelude::*;
use blockloom_core::sound::{
    MAX_VOICES, MAX_VOICES_PER_SOUND, SoundBus, SoundMixer, clamp_gain, clamp_pitch, distance_gain,
    gain_to_user,
};
use blockloom_core::soundscape::{
    Acoustics, DuckTrigger, PathEffect, ReverbSpec, SoundZoneSpec, Soundscape, WeatherSource,
    bus_is_affected, cutoff_for,
};
use std::sync::Arc;

use crate::sound_fx::{FxSource, RawIr, VoiceFx};
use blockloom_core::vm::Effect;
use blockloom_protocol::RuntimeMessage;
use std::collections::{HashMap, VecDeque};

use crate::bridge;
use crate::engine::{Dimension, Engine, PendingEffects};
use crate::world::WorldCamera;

/// Marks a spawned voice entity, so a rebuild can find every voice even
/// though none of them is an actor.
#[derive(Component)]
pub struct VoiceTag(pub u64);

/// One live voice: what to play, how loud, and where.
#[derive(Debug, Clone)]
struct Voice {
    /// Who asked, for the run log when its file won't load.
    owner: String,
    asset: String,
    bus: SoundBus,
    volume: f32,
    pitch: f32,
    /// The actor a positional voice follows, if any.
    at: Option<String>,
    position: Option<Vec3>,
    /// The soundscape weather loop driving this voice's volume, if any.
    weather: Option<usize>,
    entity: Entity,
    /// The file, until it has loaded and the voice can start playing.
    pending: Option<Handle<AudioSource>>,
    looped: bool,
    /// What the main thread asks of the voice's effects each frame.
    fx: Arc<VoiceFx>,
    /// What walls between the voice and the listener do to it.
    path: PathEffect,
    /// Where the probe last said `path` should be; the voice eases to it.
    path_goal: PathEffect,
}

/// What a new voice is made of.
struct NewVoice {
    owner: String,
    asset: String,
    bus: SoundBus,
    volume: f32,
    pitch: f32,
    looped: bool,
    at: Option<String>,
    position: Option<Vec3>,
    weather: Option<usize>,
    /// Where a positional voice starts, if it is one.
    start: Option<Vec3>,
}

/// Everything about the run's audio that isn't an entity.
#[derive(Resource, Default)]
pub struct SoundState {
    next: u64,
    /// Voice ids oldest-first, for stealing when a file is overplayed.
    order: VecDeque<u64>,
    voices: HashMap<u64, Voice>,
    /// Asset handles by project-relative path, so a file played twice loads
    /// once.
    cache: HashMap<String, Handle<AudioSource>>,
    /// The live gains, seeded from the saved mix on every rebuild.
    pub mixer: SoundMixer,
    /// The saved rules, reseeded on every rebuild.
    soundscape: Soundscape,
    /// One 0-1 duck level per rule, and the underwater muffle's.
    duck_levels: Vec<f32>,
    muffle_level: f32,
    /// Files that failed to load, so a weather loop isn't respawned at one.
    failed: std::collections::HashSet<String>,
    /// The listener's reverb and muffle after every zone, and the buses
    /// they reach.
    acoustics: [Acoustics; SoundBus::ALL.len()],
    /// Impulse responses loaded so far by path; `None` for one that failed.
    ir_cache: HashMap<String, Option<Arc<RawIr>>>,
    /// The recorded room in force right now, if any.
    ir: [Option<Arc<RawIr>>; SoundBus::ALL.len()],
    /// `set reverb to`, for the rest of the run.
    reverb_override: Option<ReverbSpec>,
    /// Where the listener is, for the occlusion probe.
    listener: Option<Vec3>,
}

impl SoundState {
    pub(crate) fn bounce(
        &mut self,
        commands: &mut Commands,
        assets: &AssetServer,
        dir: Option<&std::path::Path>,
        path: &str,
        position: Vec3,
    ) {
        self.make_room(commands, path);
        let handle = self
            .cache
            .entry(path.to_string())
            .or_insert_with(|| assets.load(crate::world::asset_path(dir, path)))
            .clone();
        self.add_voice(
            commands,
            handle,
            NewVoice {
                owner: "debris".into(),
                asset: path.into(),
                bus: SoundBus::Sfx,
                volume: 1.0,
                pitch: 1.0,
                looped: false,
                at: None,
                position: Some(position),
                weather: None,
                start: Some(position),
            },
        );
    }

    /// Spawns the voice's entity and records it. The sound itself starts
    /// once its file has loaded (`start_ready`), through its own effects.
    fn add_voice(&mut self, commands: &mut Commands, handle: Handle<AudioSource>, new: NewVoice) {
        let id = self.next;
        self.next += 1;
        let mut spawn = commands.spawn(VoiceTag(id));
        if let Some(start) = new.start {
            spawn.insert(Transform::from_translation(start));
        }
        let entity = spawn.id();
        self.order.push_back(id);
        self.voices.insert(
            id,
            Voice {
                owner: new.owner,
                asset: new.asset,
                bus: new.bus,
                volume: new.volume,
                pitch: new.pitch,
                at: new.at,
                position: new.position,
                weather: new.weather,
                entity,
                pending: Some(handle),
                looped: new.looped,
                fx: Arc::new(VoiceFx::default()),
                path: PathEffect {
                    gain: 1.0,
                    muffle: 0.0,
                },
                path_goal: PathEffect {
                    gain: 1.0,
                    muffle: 0.0,
                },
            },
        );
    }

    /// The cached handle for a project-relative sound, loading it once.
    fn handle_for(
        &mut self,
        assets: &AssetServer,
        dir: Option<&std::path::Path>,
        path: &str,
    ) -> Handle<AudioSource> {
        self.cache
            .entry(path.to_string())
            .or_insert_with(|| {
                let resolved = match dir {
                    Some(dir) => blockloom_core::assets::resolve(dir, path)
                        .map(|resolved| resolved.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string()),
                    None => path.to_string(),
                };
                assets.load::<AudioSource>(resolved)
            })
            .clone()
    }

    /// Forgets every voice and reseeds the mix from the document. A rebuild
    /// starts from silence at the saved levels.
    pub fn reset(&mut self, mixer: SoundMixer, soundscape: Soundscape) {
        self.next = 0;
        self.order.clear();
        self.voices.clear();
        self.cache.clear();
        self.mixer = mixer;
        self.failed.clear();
        self.ir_cache.clear();
        self.ir = Default::default();
        self.reverb_override = None;
        self.acoustics = Default::default();
        self.listener = None;
        self.duck_levels = vec![0.0; soundscape.ducking.len()];
        self.muffle_level = 0.0;
        self.soundscape = soundscape;
    }

    /// The soundscape's say over one bus right now: every duck aimed at it
    /// and the underwater muffle, multiplied.
    fn bus_factor(&self, bus: SoundBus) -> f32 {
        let ducks: f32 = self
            .soundscape
            .ducking
            .iter()
            .zip(&self.duck_levels)
            .filter(|(rule, _)| rule.target == SoundBus::Master || rule.target == bus)
            .map(|(rule, level)| rule.factor(*level))
            .product();
        ducks * self.soundscape.underwater.factor(bus, self.muffle_level)
    }

    /// Every asset path with a live voice, for the `is playing` reporter.
    pub fn playing(&self) -> std::collections::HashSet<String> {
        self.voices
            .values()
            .map(|voice| voice.asset.clone())
            .collect()
    }

    /// The live gains in 0-100 block scale, for the `bus volume` reporter.
    pub fn bus_volumes(&self) -> HashMap<SoundBus, f32> {
        SoundBus::ALL
            .iter()
            .map(|bus| (*bus, gain_to_user(self.mixer.gain(*bus)) as f32))
            .collect()
    }

    /// The gain a voice actually plays at, before distance: its own volume
    /// through its bus through master.
    fn effective(&self, voice: &Voice) -> f32 {
        self.mixer.master_volume
            * self.mixer.gain(voice.bus)
            * self.bus_factor(voice.bus)
            * voice.volume
    }

    fn drop_voice(&mut self, commands: &mut Commands, id: u64) {
        if let Some(voice) = self.voices.remove(&id)
            && let Ok(mut entity) = commands.get_entity(voice.entity)
        {
            entity.despawn();
        }
        self.order.retain(|candidate| *candidate != id);
    }

    /// Makes room for one more voice of `asset`, stealing the oldest of that
    /// file first and the oldest voice of all as a backstop.
    fn make_room(&mut self, commands: &mut Commands, asset: &str) {
        while self
            .voices
            .values()
            .filter(|voice| voice.asset == asset)
            .count()
            >= MAX_VOICES_PER_SOUND
        {
            let Some(oldest) = self
                .order
                .iter()
                .find(|id| {
                    self.voices
                        .get(*id)
                        .is_some_and(|voice| voice.asset == asset && voice.weather.is_none())
                })
                .copied()
            else {
                break;
            };
            self.drop_voice(commands, oldest);
        }
        while self.voices.len() >= MAX_VOICES {
            let Some(oldest) = self
                .order
                .iter()
                .find(|id| self.voices.get(*id).is_some_and(|v| v.weather.is_none()))
                .copied()
            else {
                break;
            };
            self.drop_voice(commands, oldest);
        }
    }
}

/// Turns this tick's sound effects into voices. Runs in the fixed-step chain
/// rather than in `apply_common` because sounds ignore the pause freeze: a
/// pause menu's click still clicks, and the soundtrack plays on.
pub fn apply_sound_effects(
    mut commands: Commands,
    effects: Res<PendingEffects>,
    engine: NonSend<Engine>,
    mut sound: ResMut<SoundState>,
    assets: Res<AssetServer>,
    transforms: Query<&Transform>,
) {
    for effect in &effects.0 {
        match effect {
            Effect::PlaySound {
                actor,
                sound: path,
                volume,
                pitch,
                loop_,
                bus,
                at,
            } => {
                // Positional means following a real actor; a target with no
                // entity behind it was deleted this tick, so the voice plays
                // global rather than dropping what the blocks asked for.
                let follows = at
                    .as_deref()
                    .filter(|target| engine.entities.contains_key(*target))
                    .map(str::to_string);
                let start_at = follows.as_deref().map(|target| {
                    engine
                        .entities
                        .get(target)
                        .and_then(|entity| transforms.get(*entity).ok())
                        .map(|transform| transform.translation)
                        .unwrap_or(Vec3::ZERO)
                });
                sound.make_room(&mut commands, path);
                let handle = sound.handle_for(&assets, engine.project_dir.as_deref(), path);
                sound.add_voice(
                    &mut commands,
                    handle,
                    NewVoice {
                        owner: actor.clone(),
                        asset: path.clone(),
                        bus: *bus,
                        volume: clamp_gain(*volume),
                        pitch: clamp_pitch(*pitch),
                        looped: *loop_,
                        at: follows,
                        position: None,
                        weather: None,
                        start: start_at,
                    },
                );
            }
            Effect::StopSound { sound: wanted, .. } => {
                let ids: Vec<u64> = sound
                    .voices
                    .iter()
                    .filter(|(_, voice)| {
                        voice.weather.is_none() && (wanted.is_empty() || voice.asset == *wanted)
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids {
                    sound.drop_voice(&mut commands, id);
                }
            }
            Effect::SetSoundVolume {
                sound: wanted,
                volume,
                ..
            } => {
                for voice in sound.voices.values_mut() {
                    if voice.asset == *wanted {
                        voice.volume = clamp_gain(*volume);
                    }
                }
            }
            Effect::SetSoundPitch {
                sound: wanted,
                pitch,
                ..
            } => {
                for voice in sound.voices.values_mut() {
                    if voice.asset == *wanted {
                        voice.pitch = clamp_pitch(*pitch);
                    }
                }
            }
            Effect::SetBusVolume { bus, volume } => {
                sound.mixer.set_gain(*bus, *volume);
            }
            Effect::SetReverb { preset } => match ReverbSpec::preset(preset) {
                Some(spec) => sound.reverb_override = Some(spec),
                None => bridge::send(&RuntimeMessage::Error {
                    actor: String::new(),
                    message: format!("there's no reverb preset called \"{preset}\""),
                }),
            },
            Effect::SetUnderwaterMuffle { bus, enabled } => {
                let buses = &mut sound.soundscape.underwater.buses;
                buses.retain(|b| b != bus);
                if *enabled {
                    buses.push(*bus);
                }
            }
            Effect::Stopped => {
                let ids: Vec<u64> = sound.voices.keys().copied().collect();
                for id in ids {
                    sound.drop_voice(&mut commands, id);
                }
            }
            _ => {}
        }
    }
}

/// Steps the soundscape: duck and muffle levels, the listener's zones, and
/// the weather loops' voices and volumes. Runs on the real clock so a pause
/// still ducks.
pub fn drive_soundscape(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut sound: ResMut<SoundState>,
    assets: Res<AssetServer>,
    time: Res<Time<Real>>,
    dimension: Res<Dimension>,
    atmosphere: Option<Res<crate::atmosphere::Atmosphere>>,
    water: Option<Res<crate::water::WaterSample>>,
    cameras: Query<&Transform, With<WorldCamera>>,
    placed: Query<(&crate::engine::ActorId, &Transform), Without<WorldCamera>>,
) {
    let dt = time.delta_secs();
    let sound = &mut *sound;
    let eye = cameras.iter().next().map(|t| t.translation);
    sound.listener = eye;

    // Zones: the project's ambience with each zone laid over it by how far
    // the listener is inside.
    let base = Acoustics {
        reverb: sound.reverb_override.unwrap_or(sound.soundscape.reverb),
        muffle: 0.0,
        // A `set reverb to` preset is parametric and replaces the recording.
        impulse: if sound.reverb_override.is_some() {
            String::new()
        } else {
            sound.soundscape.reverb_impulse.clone()
        },
    };
    let mut zones = Vec::new();
    let default_zone = SoundZoneSpec::default();
    if let Some(eye) = eye {
        let flat = !dimension.0.is_3d();
        for (id, transform) in &placed {
            if !engine.has_component(&id.0, "SoundZone") {
                continue;
            }
            let zone = engine
                .actor(&id.0)
                .and_then(|actor| actor.components.sound_zone())
                .unwrap_or(&default_zone);
            if !zone.enabled {
                continue;
            }
            let pose = blockloom_core::volume::VolumePose {
                position: transform.translation.to_array(),
                rotation: transform.rotation.to_array(),
                scale: transform.scale.to_array(),
            };
            let weight = zone.coverage(&pose, eye.to_array(), flat);
            if weight > 0.0 {
                zones.push((weight, &id.0, zone));
            }
        }
    }
    zones.sort_unstable_by(|a, b| {
        a.2.priority
            .total_cmp(&b.2.priority)
            .then_with(|| a.1.cmp(b.1))
    });
    for (i, &bus) in SoundBus::ALL.iter().enumerate() {
        let mut room = if bus_is_affected(&sound.soundscape.reverb_buses, bus) {
            base.clone()
        } else {
            Acoustics::default()
        };
        for &(weight, _, zone) in &zones {
            room.apply_zone(bus, weight, zone);
        }
        sound.ir[i] = if room.reverb.is_off() || room.impulse.is_empty() {
            None
        } else {
            sound
                .ir_cache
                .entry(room.impulse.clone())
                .or_insert_with(|| load_impulse(engine.project_dir.as_deref(), &room.impulse))
                .clone()
        };
        sound.acoustics[i] = room;
    }

    if sound.soundscape.is_empty() {
        return;
    }
    for (i, rule) in sound.soundscape.ducking.iter().enumerate() {
        let active = match &rule.trigger {
            DuckTrigger::Paused => engine.running && engine.paused,
            DuckTrigger::Sound(path) => sound.voices.values().any(|voice| {
                voice.weather.is_none() && voice.pending.is_none() && voice.asset == *path
            }),
        };
        if let Some(level) = sound.duck_levels.get_mut(i) {
            *level = rule.step(*level, active, dt);
        }
    }
    let under = match (&water, eye) {
        (Some(water), Some(eye)) => water.0.underwater(eye.to_array()),
        _ => false,
    };
    sound.muffle_level = sound
        .soundscape
        .underwater
        .step(sound.muffle_level, under, dt);

    if !engine.running {
        return;
    }

    for i in 0..sound.soundscape.weather.len() {
        let spec = sound.soundscape.weather[i].clone();
        let value = atmosphere.as_ref().map_or(0.0, |a| match spec.source {
            WeatherSource::Wind => a.0.wind_speed,
            WeatherSource::Rain => a.0.rain,
            WeatherSource::Snow => a.0.snow,
            WeatherSource::Storm => a.0.storm,
        });
        let gain = spec.gain(value);
        let existing = sound
            .voices
            .iter_mut()
            .find(|(_, voice)| voice.weather == Some(i));
        if let Some((_, voice)) = existing {
            voice.volume = gain;
            continue;
        }
        if spec.sound.is_empty()
            || sound.failed.contains(&spec.sound)
            || sound.voices.len() >= MAX_VOICES - 1
            || sound
                .voices
                .values()
                .filter(|voice| voice.asset == spec.sound)
                .count()
                >= MAX_VOICES_PER_SOUND - 1
        {
            continue;
        }
        let handle = sound.handle_for(&assets, engine.project_dir.as_deref(), &spec.sound);
        sound.add_voice(
            &mut commands,
            handle,
            NewVoice {
                owner: "soundscape".into(),
                asset: spec.sound.clone(),
                bus: spec.bus,
                volume: gain,
                pitch: 1.0,
                looped: true,
                at: None,
                position: None,
                weather: Some(i),
                start: None,
            },
        );
    }
}

/// Reads a project-relative WAV as an impulse response, reporting a failure
/// once (the cache keeps the `None`).
fn load_impulse(dir: Option<&std::path::Path>, path: &str) -> Option<Arc<RawIr>> {
    let resolved = match dir {
        Some(dir) => blockloom_core::assets::resolve(dir, path)?,
        None => std::path::PathBuf::from(path),
    };
    let loaded = blockloom_core::vfs::read(&resolved)
        .map_err(|e| e.to_string())
        .and_then(|bytes| RawIr::from_wav(&bytes));
    match loaded {
        Ok(ir) => Some(Arc::new(ir)),
        Err(why) => {
            bridge::send(&RuntimeMessage::Error {
                actor: String::new(),
                message: format!("couldn't load reverb impulse \"{path}\": {why}"),
            });
            None
        }
    }
}

/// What walls do to each positional voice on its way to the listener: casts
/// the line from the voice to the listener against static scenery a few
/// times a second, and when it is blocked looks for a way round an edge.
pub fn probe_occlusion(
    engine: NonSend<Engine>,
    mut sound: ResMut<SoundState>,
    dimension: Res<Dimension>,
    queries: crate::queries::QueryAccess,
    transforms: Query<&Transform, Without<VoiceTag>>,
    mut frame: Local<u32>,
) {
    let occlusion = sound.soundscape.occlusion;
    if !occlusion.enabled || !engine.running {
        return;
    }
    // About ten probes a second at 60 fps; voices move slowly next to that.
    *frame = frame.wrapping_add(1);
    if !frame.is_multiple_of(6) {
        return;
    }
    let Some(ear) = sound.listener else {
        return;
    };
    let is_3d = dimension.0.is_3d();
    let sound = &mut *sound;
    queries.scope(0, || {
        for voice in sound.voices.values_mut() {
            let source = voice.position.or_else(|| {
                voice
                    .at
                    .as_ref()
                    .and_then(|target| engine.entities.get(target))
                    .and_then(|entity| transforms.get(*entity).ok())
                    .map(|transform| transform.translation)
            });
            let Some(source) = source else {
                voice.path_goal = PathEffect {
                    gain: 1.0,
                    muffle: 0.0,
                };
                continue;
            };
            voice.path_goal = path_effect(&occlusion, source, ear, is_3d, &walls_between);
        }
    });
}

/// Static scenery crossed by the segment: bodies and triggers never block.
fn walls_between(from: Vec3, to: Vec3) -> u32 {
    use blockloom_core::physics::query::{QueryFilter, QueryRequest, TriggerPolicy, dispatch};
    let request = QueryRequest::Ray {
        from: from.to_array(),
        to: to.to_array(),
        all: true,
    };
    let filter = QueryFilter {
        triggers: TriggerPolicy::Ignore,
        ..QueryFilter::default()
    };
    let length = from.distance(to);
    let out = dispatch(&request, &filter, 16);
    out.hits
        .iter()
        .filter(|hit| hit.body.is_none() && !hit.trigger)
        // A hit right at either end is the emitter's own surface or the
        // listener's, not a wall between them.
        .filter(|hit| hit.distance > 0.05 && hit.distance < length - 0.05)
        .count() as u32
}

/// The effect of the path from `from` to `to`, given a wall counter for a
/// segment. A blocked line that has a clear two-leg detour round an edge is
/// obstruction (dull, not quiet); otherwise each wall costs gain and highs.
fn path_effect(
    occlusion: &blockloom_core::soundscape::Occlusion,
    from: Vec3,
    to: Vec3,
    is_3d: bool,
    walls: &dyn Fn(Vec3, Vec3) -> u32,
) -> PathEffect {
    let direct = walls(from, to);
    if direct == 0 {
        return occlusion.effect(0, false);
    }
    let mut bends = false;
    if occlusion.obstruction > 0.0 {
        let span = to - from;
        let length = span.length();
        let dir = span / length.max(1e-6);
        let mid = (from + to) * 0.5;
        let step = (length * 0.25).clamp(if is_3d { 1.0 } else { 40.0 }, f32::MAX);
        let mut sides = Vec::new();
        if is_3d {
            let side = dir.cross(Vec3::Y).normalize_or_zero();
            sides.extend([side, -side, Vec3::Y]);
        } else {
            let side = Vec3::new(-dir.y, dir.x, 0.0);
            sides.extend([side, -side]);
        }
        'search: for scale in [1.0, 2.0] {
            for side in &sides {
                let via = mid + *side * step * scale;
                if walls(from, via) == 0 && walls(via, to) == 0 {
                    bends = true;
                    break 'search;
                }
            }
        }
    }
    occlusion.effect(direct, bends)
}

/// Follows positional targets, applies per-frame gains, and reaps finished
/// and failed voices. One pass rather than three because every step wants
/// the same lookups, and voice counts stay small.
pub fn maintain_voices(
    mut commands: Commands,
    engine: NonSend<Engine>,
    mut sound: ResMut<SoundState>,
    assets: Res<AssetServer>,
    dimension: Res<Dimension>,
    // Voices excluded: they are moved through `voice_transforms` below, and
    // two queries may not share `Transform` with one of them mutable.
    transforms: Query<&Transform, Without<VoiceTag>>,
    // The listener, which is never a voice for the same reason.
    cameras: Query<&Transform, (With<WorldCamera>, Without<VoiceTag>)>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial_sinks: Query<&mut SpatialAudioSink>,
    mut voice_transforms: Query<(&mut Transform, &VoiceTag)>,
    loaded: Res<Assets<AudioSource>>,
    mut fx_assets: Option<ResMut<Assets<FxSource>>>,
    time: Option<Res<Time<Real>>>,
) {
    // Probes are sampled; the voices glide between their answers so a wall
    // appearing or a corner turning never clicks.
    let ease = 1.0 - (-time.map_or(0.016, |t| t.delta_secs()) * 10.0).exp();
    for voice in sound.voices.values_mut() {
        voice.path.gain += (voice.path_goal.gain - voice.path.gain) * ease;
        voice.path.muffle += (voice.path_goal.muffle - voice.path.muffle) * ease;
    }
    let is_3d = dimension.0.is_3d();
    let listener = cameras.iter().next().map(|transform| transform.translation);
    // Owned up front: the pass below mutates the state (failed loads drop
    // their handle, finished voices reap) while reading it.
    let snapshot: Vec<(u64, Voice)> = sound
        .voices
        .iter()
        .map(|(id, voice)| (*id, voice.clone()))
        .collect();
    let mut reap: Vec<u64> = Vec::new();
    for (id, voice) in &snapshot {
        // A positional voice whose actor left the world ends with it.
        if let Some(target) = &voice.at
            && !engine.entities.contains_key(target)
        {
            reap.push(*id);
            continue;
        }
        // A file that won't load complains once, against whoever asked, and
        // takes every voice waiting on it with it: without their handle
        // they would stall forever, loaded neither dead nor alive.
        if let Some(handle) = sound.cache.get(&voice.asset)
            && matches!(assets.load_state(handle), bevy::asset::LoadState::Failed(_))
        {
            bridge::send(&RuntimeMessage::Error {
                actor: voice.owner.clone(),
                message: format!("couldn't load sound \"{}\"", voice.asset),
            });
            sound.cache.remove(&voice.asset);
            sound.failed.insert(voice.asset.clone());
            reap.extend(
                snapshot
                    .iter()
                    .filter(|(_, other)| other.asset == voice.asset)
                    .map(|(id, _)| *id),
            );
            continue;
        }
        // Effects: what the listener's zones, the water and the walls in
        // the way ask of this voice's bus.
        {
            let sunk = 1.0
                - sound
                    .soundscape
                    .underwater
                    .factor(voice.bus, sound.muffle_level);
            let bus = match voice.bus {
                SoundBus::Master => 0,
                SoundBus::Music => 1,
                SoundBus::Sfx => 2,
            };
            let room = &sound.acoustics[bus];
            let total = 1.0 - (1.0 - sunk) * (1.0 - room.muffle) * (1.0 - voice.path.muffle);
            voice.fx.set_cutoff(cutoff_for(total));
            voice.fx.set_impulse(sound.ir[bus].as_ref());
            let reverb = room.reverb;
            voice.fx.set_reverb(
                reverb.mix,
                reverb.size,
                reverb.decay,
                reverb.damping,
                reverb.excursion,
            );
        }
        let mut gain = sound.effective(voice) * voice.path.gain;
        if let Some(position) = voice.position
            && let Some(ear) = listener
        {
            gain *= distance_gain(position.distance(ear), is_3d);
        }
        if let Some(target) = &voice.at {
            let (emitter, ear) = match (
                engine
                    .entities
                    .get(target)
                    .and_then(|entity| transforms.get(*entity).ok()),
                listener,
            ) {
                (Some(emitter), Some(ear)) => (emitter.translation, ear),
                _ => continue,
            };
            gain *= distance_gain(emitter.distance(ear), is_3d);
            if let Ok((mut transform, tag)) = voice_transforms.get_mut(voice.entity) {
                debug_assert_eq!(tag.0, *id);
                transform.translation = emitter;
            }
        }
        // Once the file is in, the voice gets its own effect chain and
        // starts. Until then it holds no sink and costs nothing.
        if let Some(handle) = &voice.pending
            && let Some(file) = loaded.get(handle)
            && let Some(fx_assets) = fx_assets.as_mut()
        {
            let source = fx_assets.add(FxSource {
                source: file.clone(),
                fx: voice.fx.clone(),
                looped: voice.looped,
            });
            commands.entity(voice.entity).insert((
                AudioPlayer(source),
                PlaybackSettings {
                    volume: Volume::Linear(gain.max(0.0)),
                    speed: voice.pitch,
                    spatial: voice.at.is_some() || voice.position.is_some(),
                    ..PlaybackSettings::ONCE
                },
            ));
            if let Some(live) = sound.voices.get_mut(id) {
                live.pending = None;
            }
        }
        if let Ok(mut sink) = sinks.get_mut(voice.entity) {
            sink.set_volume(Volume::Linear(gain.max(0.0)));
            sink.set_speed(voice.pitch);
            if sink.empty() {
                reap.push(*id);
            }
        } else if let Ok(mut sink) = spatial_sinks.get_mut(voice.entity) {
            sink.set_volume(Volume::Linear(gain.max(0.0)));
            sink.set_speed(voice.pitch);
            if sink.empty() {
                reap.push(*id);
            }
        }
    }
    for id in reap {
        sound.drop_voice(&mut commands, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::InsertEngine;
    use blockloom_core::scene::Mode;

    fn app() -> App {
        // Asset loads run on the IO pool, which the test harness sets up the
        // way the plugins would in a running game.
        bevy::tasks::IoTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let (_sender, incoming) = std::sync::mpsc::channel();
        let engine = Engine::new(incoming, Mode::TwoD);
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<AudioSource>();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Dimension(Mode::TwoD));
        app.init_resource::<PendingEffects>();
        app.init_resource::<SoundState>();
        app.insert_engine(engine);
        app
    }

    fn play(actor: &str) -> Effect {
        Effect::PlaySound {
            actor: actor.to_string(),
            sound: "assets/sounds/kick.wav".to_string(),
            volume: 0.8,
            pitch: 1.0,
            loop_: false,
            bus: SoundBus::Sfx,
            at: None,
        }
    }

    #[test]
    fn the_sound_systems_run_with_nothing_to_do() {
        // Headless schedule check: conflicting queries panic when the system
        // runs, not when it is added.
        let mut app = app();
        app.add_systems(Update, (apply_sound_effects, maintain_voices).chain());
        app.update();
        app.update();
    }

    #[test]
    fn a_play_spawns_a_voice_and_a_stop_clears_it() {
        let mut app = app();
        app.add_systems(Update, apply_sound_effects);

        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(play("a1"));
        app.update();
        let sound = app.world().resource::<SoundState>();
        assert_eq!(sound.voices.len(), 1);
        assert_eq!(sound.playing().len(), 1);
        let entity = sound.voices.values().next().expect("one voice").entity;
        assert!(app.world().get::<VoiceTag>(entity).is_some());

        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::StopSound {
                actor: "a1".to_string(),
                sound: String::new(),
            });
        app.update();
        assert!(app.world().resource::<SoundState>().voices.is_empty());
    }

    #[test]
    fn a_bus_move_rescales_the_reported_gains() {
        let mut app = app();
        app.add_systems(Update, apply_sound_effects);

        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::SetBusVolume {
                bus: SoundBus::Music,
                volume: 0.5,
            });
        app.update();
        let sound = app.world().resource::<SoundState>();
        assert_eq!(sound.bus_volumes().get(&SoundBus::Music), Some(&50.0));
        assert_eq!(sound.bus_volumes().get(&SoundBus::Master), Some(&100.0));
    }

    fn scape() -> Soundscape {
        use blockloom_core::soundscape::{DuckRule, WeatherLoop};
        Soundscape {
            weather: vec![WeatherLoop {
                sound: "assets/sounds/rain.ogg".into(),
                source: WeatherSource::Rain,
                from: 0.0,
                to: 1.0,
                max_gain: 1.0,
                bus: SoundBus::Sfx,
            }],
            ducking: vec![DuckRule {
                trigger: DuckTrigger::Paused,
                target: SoundBus::Music,
                amount: 0.5,
                attack: 0.0,
                release: 0.0,
            }],
            underwater: Default::default(),
            ..Soundscape::default()
        }
    }

    #[test]
    fn a_weather_loop_starts_with_the_run_and_survives_stop_sound() {
        let mut app = app();
        app.init_resource::<Time<Real>>();
        app.world_mut()
            .resource_mut::<SoundState>()
            .reset(SoundMixer::default(), scape());
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.add_systems(Update, (drive_soundscape, apply_sound_effects).chain());
        app.update();
        assert_eq!(app.world().resource::<SoundState>().voices.len(), 1);

        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .push(Effect::StopSound {
                actor: "a".into(),
                sound: String::new(),
            });
        app.update();
        assert_eq!(app.world().resource::<SoundState>().voices.len(), 1);
    }

    #[test]
    fn weather_leaves_room_for_gameplay_voices_and_skips_unfinished_rows() {
        let mut app = app();
        let mut scape = scape();
        let weather = &mut scape.weather[0];
        weather.sound = "assets/sounds/kick.wav".into();
        let mut blank = weather.clone();
        blank.sound.clear();
        scape.weather = vec![weather.clone(); MAX_VOICES_PER_SOUND + 1];
        scape.weather.insert(0, blank);
        app.init_resource::<Time<Real>>();
        app.world_mut()
            .resource_mut::<SoundState>()
            .reset(SoundMixer::default(), scape);
        app.world_mut().non_send_mut::<Engine>().running = true;
        app.add_systems(Update, (drive_soundscape, apply_sound_effects).chain());

        app.update();
        assert_eq!(
            app.world().resource::<SoundState>().voices.len(),
            MAX_VOICES_PER_SOUND - 1
        );
        app.world_mut()
            .resource_mut::<PendingEffects>()
            .0
            .extend([play("a"), play("b")]);
        app.update();
        let sound = app.world().resource::<SoundState>();
        assert_eq!(sound.voices.len(), MAX_VOICES_PER_SOUND);
        assert!(sound.voices.values().any(|voice| voice.owner == "b"));
        assert!(sound.voices.values().all(|voice| !voice.asset.is_empty()));
    }

    #[test]
    fn pausing_ducks_the_target_bus() {
        let mut app = app();
        app.init_resource::<Time<Real>>();
        app.world_mut()
            .resource_mut::<SoundState>()
            .reset(SoundMixer::default(), scape());
        {
            let mut engine = app.world_mut().non_send_mut::<Engine>();
            engine.running = true;
            engine.paused = true;
        }
        app.add_systems(Update, drive_soundscape);
        app.update();
        let sound = app.world().resource::<SoundState>();
        assert!((sound.bus_factor(SoundBus::Music) - 0.5).abs() < 1e-6);
        assert_eq!(sound.bus_factor(SoundBus::Sfx), 1.0);
    }

    #[test]
    fn a_blocked_line_with_a_clear_detour_is_obstruction() {
        use blockloom_core::soundscape::Occlusion;
        let occ = Occlusion {
            enabled: true,
            ..Occlusion::default()
        };
        // A wall on the straight line along x between 0 and 10 only.
        let wall = |a: Vec3, b: Vec3| -> u32 {
            let crosses = (a.x < 5.0) != (b.x < 5.0);
            let low = a.y.abs() < 1.0 && b.y.abs() < 1.0;
            (crosses && low) as u32
        };
        let from = Vec3::new(0.0, 0.0, 0.0);
        let to = Vec3::new(10.0, 0.0, 0.0);
        let bent = path_effect(&occ, from, to, false, &wall);
        assert_eq!(bent.gain, 1.0);
        assert!(bent.muffle > 0.0);
        // A wall nothing goes round: gain and highs both pay.
        let solid = |a: Vec3, b: Vec3| -> u32 { ((a.x < 5.0) != (b.x < 5.0)) as u32 };
        let blocked = path_effect(&occ, from, to, false, &solid);
        assert!(blocked.gain < 1.0);
        let clear = path_effect(&occ, from, to, false, &|_, _| 0);
        assert_eq!(clear.gain, 1.0);
        assert_eq!(clear.muffle, 0.0);
    }

    #[test]
    fn set_reverb_and_underwater_muffle_land_in_the_state() {
        let mut app = app();
        app.add_systems(Update, apply_sound_effects);
        {
            let mut fx = app.world_mut().resource_mut::<PendingEffects>();
            fx.0.push(Effect::SetReverb {
                preset: "Cave".into(),
            });
            fx.0.push(Effect::SetUnderwaterMuffle {
                bus: SoundBus::Music,
                enabled: true,
            });
        }
        app.update();
        let sound = app.world().resource::<SoundState>();
        assert!(sound.reverb_override.is_some_and(|r| r.decay > 3.0));
        assert_eq!(sound.soundscape.underwater.buses, vec![SoundBus::Music]);
    }
}
