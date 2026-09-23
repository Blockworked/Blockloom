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
    AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackMode, PlaybackSettings,
    SpatialAudioSink, Volume,
};
use bevy::prelude::*;
use blockloom_core::sound::{
    MAX_VOICES, MAX_VOICES_PER_SOUND, SoundBus, SoundMixer, clamp_gain, clamp_pitch, distance_gain,
    gain_to_user,
};
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
    /// The actor a positional voice follows, by id. `None` is global.
    at: Option<String>,
    entity: Entity,
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
}

impl SoundState {
    /// Forgets every voice and reseeds the mix from the document. A rebuild
    /// starts from silence at the saved levels.
    pub fn reset(&mut self, mixer: SoundMixer) {
        self.next = 0;
        self.order.clear();
        self.voices.clear();
        self.cache.clear();
        self.mixer = mixer;
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
        self.mixer.master_volume * self.mixer.gain(voice.bus) * voice.volume
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
                        .is_some_and(|voice| voice.asset == asset)
                })
                .copied()
            else {
                break;
            };
            self.drop_voice(commands, oldest);
        }
        while self.voices.len() >= MAX_VOICES {
            let Some(oldest) = self.order.front().copied() else {
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
                let start = follows
                    .as_deref()
                    .and_then(|target| engine.entities.get(target))
                    .and_then(|entity| transforms.get(*entity).ok())
                    .map(|transform| transform.translation)
                    .unwrap_or(Vec3::ZERO);
                sound.make_room(&mut commands, path);
                let handle = sound
                    .cache
                    .entry(path.clone())
                    .or_insert_with(|| {
                        let resolved = match engine.project_dir.as_deref() {
                            Some(dir) => blockloom_core::assets::resolve(dir, path)
                                .map(|resolved| resolved.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.clone()),
                            None => path.clone(),
                        };
                        assets.load::<AudioSource>(resolved)
                    })
                    .clone();
                let mode = if *loop_ {
                    PlaybackMode::Loop
                } else {
                    PlaybackMode::Once
                };
                let initial =
                    sound.mixer.master_volume * sound.mixer.gain(*bus) * clamp_gain(*volume);
                let id = sound.next;
                sound.next += 1;
                let pitch = clamp_pitch(*pitch);
                let mut spawn = commands.spawn((
                    AudioPlayer(handle),
                    PlaybackSettings {
                        mode,
                        volume: Volume::Linear(initial.max(0.0)),
                        speed: pitch,
                        spatial: follows.is_some(),
                        ..PlaybackSettings::ONCE
                    },
                    VoiceTag(id),
                ));
                if follows.is_some() {
                    spawn.insert(Transform::from_translation(start));
                }
                let entity = spawn.id();
                sound.order.push_back(id);
                sound.voices.insert(
                    id,
                    Voice {
                        owner: actor.clone(),
                        asset: path.clone(),
                        bus: *bus,
                        volume: clamp_gain(*volume),
                        pitch,
                        at: follows,
                        entity,
                    },
                );
            }
            Effect::StopSound { sound: wanted, .. } => {
                let ids: Vec<u64> = sound
                    .voices
                    .iter()
                    .filter(|(_, voice)| wanted.is_empty() || voice.asset == *wanted)
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
) {
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
            reap.extend(
                snapshot
                    .iter()
                    .filter(|(_, other)| other.asset == voice.asset)
                    .map(|(id, _)| *id),
            );
            continue;
        }
        let mut gain = sound.effective(voice);
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
        app.insert_non_send(engine);
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
}
