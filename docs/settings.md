# Settings

The project settings button opens one window with categories on the left.
General chooses the default scene. Under Publishing, App Info sets the game
icon and Android holds application identity, version and signing settings.

The right inspector follows selection. Click the scene header in the hierarchy
or a scene asset to edit its components. Actor and scene components use the same
open card layout; selecting an actor shows its components automatically.
Dimension, physics, camera, quality, sky, fog, weather, post-process, display,
sound and input settings belong to each scene. Double-click a scene asset to
open its scene and show its components.

Create a Lighting asset from the asset tray's New asset menu, or use Create
Lighting asset on the scene's Lighting card to keep its current lighting.
Select the asset in the tray to edit sun and ambient light, shadows and ray
tracing in the right inspector. Drag it onto a scene's Lighting field or hierarchy header to use it.
Dragging keeps the current inspector visible; clicking selects the asset.
Drop fields show the accepted asset type icon, such as a sun for Lighting or
an image for textures, and highlight when a compatible asset is dragged.
Editing the asset updates every scene using it. Clearing the field keeps the
current values as inline lighting. Existing scenes still load their inline
settings, and a missing Lighting file keeps the scene's last saved lighting.

Lighting files use `.blocklighting` and contain the serialized Lighting settings.
The shell/MCP commands are `read-lighting-asset path=...`,
`write-lighting-asset path=... lighting={...}` and
`set-scene-lighting-asset path=... sceneId=...`. The scene id is optional and
defaults to the active scene. An empty path detaches the selected scene.
