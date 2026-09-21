# Long-term TODO

Obvious gaps already identified in the project notes:

- [ ] Add clones so blocks can create and manage copies of an actor.
- [ ] Add sound playback and sound-related blocks.
- [ ] Add lists and blocks for creating, reading, and changing list items.
- [ ] Add asset management UI for importing, organizing, previewing, replacing, and removing project assets.
- [x] Show `say` as a speech bubble over its actor in the game world.
- [ ] Let reporter-shaped custom blocks suspend and resume when they contain `wait`.
- [x] Handle actors whose visual shape does not match the project's dimension, including a way to convert or replace the shape.
- [ ] Add project packaging so a finished game can be shared and run independently.
- [x] Redesign projects as folders: the app always starts on a Dashboard page for creating new projects and opening existing ones, with each project stored as a folder so it can hold assets.
- [x] Add a component system built on Bevy's ECS components that turns all properties into components, with support for custom components and camera-attach components (for first-person / third-person cameras).
- [x] Add a script component that runs Rust, so a project can drop out of blocks where it needs to.
- [x] Let blocks attach and detach whole components at runtime, not just write their fields.
- [ ] Ship the script toolchain, or degrade well without one: a script needs `rustc` on the machine that presses Play, and a packaged install can't assume it.
- [ ] Give the script editor real Rust editing - highlighting, and errors shown against the line they're on rather than only in the run log.
