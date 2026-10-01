# Settings

The project settings button opens a menu. General / Project chooses the default
scene. Publishing / App Info sets the game icon, and Publishing / Android holds
application identity, version and signing settings.

Use the Scene tab in the right inspector to edit the active scene's components.
Expand a card to edit it; its reset button restores the component defaults.
Dimension, physics, camera, quality, sky, fog, weather, post-process, display,
sound and input settings belong to each scene. Double-click a scene asset to
open its scene and show its components.

Create a Lighting asset from the asset tray's New asset menu, or use Create
Lighting asset on the scene's Lighting asset card to keep its current lighting.
Select the asset in the tray to edit sun and ambient light, shadows and ray
tracing in the right inspector. Drag it onto a scene's Lighting field to use it.
Editing the asset updates every scene using it. Clearing the field keeps the
current values as inline lighting. Existing scenes still load their inline
settings, and a missing Lighting file keeps the scene's last saved lighting.

Lighting files use `.blocklighting` and contain the serialized Lighting settings.
The shell/MCP commands are `read-lighting-asset path=...`,
`write-lighting-asset path=... lighting={...}` and
`set-scene-lighting-asset path=...`. An empty path detaches the active scene.
