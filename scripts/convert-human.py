"""Converts Quaternius's CC0 "Animated Human" .blend into res/models/human.glb.

Needs Blender's Python module (`pip install bpy` in a venv on a matching
Python). Run with the pack's .blend and the output path:

    BLEND=".../Blend/Animated Human.blend" OUT=res/models/human.glb python -I scripts/convert-human.py
"""

import math
import os

import bpy
from mathutils import Matrix, Vector

KEEP = ["Idle", "Walk", "Run", "Jump", "Punch", "Death"]

bpy.ops.wm.open_mainfile(filepath=os.environ["BLEND"])
arm = bpy.data.objects["Human Armature"]
mesh = bpy.data.objects["Human_Mesh"]

for action in list(bpy.data.actions):
    if action.name not in KEEP:
        bpy.data.actions.remove(action)
arm.animation_data.action = None
for track in list(arm.animation_data.nla_tracks):
    arm.animation_data.nla_tracks.remove(track)

# The pack's material is a legacy Diffuse node pointing at a missing path;
# a Principled one over the palette texture is what glTF can carry.
material = bpy.data.materials["Texture"]
nodes = material.node_tree
nodes.nodes.clear()
out = nodes.nodes.new("ShaderNodeOutputMaterial")
bsdf = nodes.nodes.new("ShaderNodeBsdfPrincipled")
image = nodes.nodes.new("ShaderNodeTexImage")
skin = os.environ.get("SKIN", "ClothedLightSkin.png")
image.image = bpy.data.images.load(os.path.join(os.path.dirname(os.environ["BLEND"]), "Textures", skin))
image.interpolation = "Closest"
bsdf.inputs["Roughness"].default_value = 0.85
nodes.links.new(image.outputs["Color"], bsdf.inputs["Base Color"])
nodes.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])

# One muted NLA track per clip, so the exporter writes each as an animation.
for name in KEEP:
    track = arm.animation_data.nla_tracks.new()
    track.name = name
    track.strips.new(name, 0, bpy.data.actions[name]).name = name
    track.mute = True

# 1.8 m tall around the origin, toes along Blender +Y (glTF -Z).
corners = [mesh.matrix_world @ Vector(c) for c in mesh.bound_box]
low = min(c.z for c in corners)
high = max(c.z for c in corners)
toe = arm.matrix_world @ arm.data.bones["LeftToe_End"].head_local
hips = arm.matrix_world @ arm.data.bones["Hips"].head_local
k = 1.8 / (high - low)
turn = Matrix.Identity(4) if toe.y > hips.y else Matrix.Rotation(math.pi, 4, "Z")
arm.matrix_world = Matrix.Translation((0, 0, -0.9 - low * k)) @ turn @ Matrix.Scale(k, 4) @ arm.matrix_world
bpy.context.view_layer.update()

bpy.ops.export_scene.gltf(
    filepath=os.environ["OUT"],
    export_format="GLB",
    export_animations=True,
    export_animation_mode="NLA_TRACKS",
    export_force_sampling=True,
    export_frame_step=1,
    export_yup=True,
    export_apply=False,
    export_skins=True,
    export_morph=False,
    export_cameras=False,
    export_lights=False,
)
