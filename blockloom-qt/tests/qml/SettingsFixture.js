function world() { return {
    "background": "#1B2431",
    "camera": {
        "look_at": [
            0.0,
            0.0,
            0.0
        ],
        "position": [
            0.0,
            6.0,
            14.0
        ],
        "zoom": 1.0
    },
    "cloud_layers": [],
    "clouds": {
        "ambient": 1.0,
        "anvil": 0.30000001192092896,
        "back_blend": 0.20000000298023224,
        "backward": -0.20000000298023224,
        "billow": 0.15000000596046448,
        "bottom": 1500.0,
        "bottom_occlusion": 0.6499999761581421,
        "cloud_type": 0.699999988079071,
        "coverage": 0.5,
        "density": 0.800000011920929,
        "detail_scale": 6.0,
        "detail_speed": 1.0,
        "detail_volume": "",
        "enabled": false,
        "erosion": 0.3499999940395355,
        "feather": 0.20000000298023224,
        "forward": 0.800000011920929,
        "lightbleed": 0.30000001192092896,
        "moon_shadows": true,
        "powder": 1.0,
        "quality": "High",
        "shadow_range": 20000.0,
        "shadow_strength": 0.699999988079071,
        "shadows": true,
        "shape_volume": "",
        "shear": [
            0.0,
            0.0
        ],
        "shoulder": 0.800000011920929,
        "sun_shadows": true,
        "threshold": 0.009999999776482582,
        "tiling_km": 12.0,
        "toe": 0.20000000298023224,
        "top": 3500.0
    },
    "cutscenes": [],
    "director": {
        "aurora_kp": {
            "keys": [],
            "loop_enabled": false
        },
        "cloud_coverage": {
            "keys": [],
            "loop_enabled": false
        },
        "cloud_type": {
            "keys": [],
            "loop_enabled": false
        },
        "day_length": 600.0,
        "enabled": false,
        "exposure": {
            "keys": [],
            "loop_enabled": false
        },
        "fog_density": {
            "keys": [],
            "loop_enabled": false
        },
        "loop_enabled": true,
        "lut_weight": {
            "keys": [],
            "loop_enabled": false
        },
        "moon_azimuth": {
            "keys": [],
            "loop_enabled": false
        },
        "moon_elevation": {
            "keys": [],
            "loop_enabled": false
        },
        "precipitation": {
            "keys": [],
            "loop_enabled": false
        },
        "presets": [],
        "sun_azimuth": {
            "keys": [],
            "loop_enabled": false
        },
        "sun_elevation": {
            "keys": [],
            "loop_enabled": false
        },
        "temperature": {
            "keys": [],
            "loop_enabled": false
        },
        "time_of_day": 12.0,
        "wetness": {
            "keys": [],
            "loop_enabled": false
        },
        "wind_direction": {
            "keys": [],
            "loop_enabled": false
        },
        "wind_speed": {
            "keys": [],
            "loop_enabled": false
        }
    },
    "display": {
        "paper_white_nits": 200.0,
        "peak_nits": 1000.0,
        "space": "Sdr"
    },
    "fixed_rate": 60.0,
    "fog": {
        "aerial": {
            "blue_shift": 0.699999988079071,
            "desaturation": 0.4000000059604645,
            "distance": 8000.0,
            "enabled": false,
            "height_scale": 1200.0,
            "tint": "#FFFFFF"
        },
        "height": {
            "base_height": 0.0,
            "day_color": "#C2CAD2",
            "distance": 400.0,
            "dusk_color": "#E8A778",
            "enabled": false,
            "falloff": 0.05000000074505806,
            "night_color": "#2A3344",
            "start": 0.0,
            "sun_boost": 0.5,
            "sun_boost_g": 0.75
        },
        "volumetric": {
            "albedo": "#FFFFFF",
            "ambient": 1.0,
            "anisotropy": 0.6000000238418579,
            "base_height": 0.0,
            "density": 0.019999999552965164,
            "dust": {
                "alpha": 0.6000000238418579,
                "count": 600,
                "drift": 0.05000000074505806,
                "enabled": false,
                "size": 0.012000000104308128,
                "twinkle": 0.5
            },
            "dust_height": 3.0,
            "emissive": "#000000",
            "emissive_strength": 0.0,
            "enabled": false,
            "falloff": 0.10000000149011612,
            "noise": 0.5,
            "noise_scale": 12.0,
            "noise_wind": [
                1.0,
                0.0,
                0.5
            ],
            "quality": "Medium",
            "range": 96.0,
            "sun": true
        }
    },
    "gravity": [
        0.0,
        -981.0,
        0.0
    ],
    "input": {
        "actions": [
            {
                "bindings": [
                    {
                        "binding": "Key",
                        "key": "space"
                    },
                    {
                        "binding": "GamepadButton",
                        "button": "south"
                    }
                ],
                "name": "Jump"
            },
            {
                "bindings": [
                    {
                        "binding": "Mouse",
                        "button": "Left"
                    },
                    {
                        "binding": "GamepadButton",
                        "button": "righttrigger"
                    }
                ],
                "name": "Fire"
            },
            {
                "bindings": [
                    {
                        "binding": "Key",
                        "key": "a"
                    },
                    {
                        "binding": "Key",
                        "key": "left arrow"
                    },
                    {
                        "binding": "GamepadButton",
                        "button": "dpadleft"
                    },
                    {
                        "axis": "leftstickx",
                        "binding": "GamepadAxis",
                        "direction": -1
                    }
                ],
                "name": "Left"
            },
            {
                "bindings": [
                    {
                        "binding": "Key",
                        "key": "d"
                    },
                    {
                        "binding": "Key",
                        "key": "right arrow"
                    },
                    {
                        "binding": "GamepadButton",
                        "button": "dpadright"
                    },
                    {
                        "axis": "leftstickx",
                        "binding": "GamepadAxis",
                        "direction": 1
                    }
                ],
                "name": "Right"
            },
            {
                "bindings": [
                    {
                        "binding": "Key",
                        "key": "w"
                    },
                    {
                        "binding": "Key",
                        "key": "up arrow"
                    },
                    {
                        "binding": "GamepadButton",
                        "button": "dpadup"
                    },
                    {
                        "axis": "leftsticky",
                        "binding": "GamepadAxis",
                        "direction": 1
                    }
                ],
                "name": "Up"
            },
            {
                "bindings": [
                    {
                        "binding": "Key",
                        "key": "s"
                    },
                    {
                        "binding": "Key",
                        "key": "down arrow"
                    },
                    {
                        "binding": "GamepadButton",
                        "button": "dpaddown"
                    },
                    {
                        "axis": "leftsticky",
                        "binding": "GamepadAxis",
                        "direction": -1
                    }
                ],
                "name": "Down"
            }
        ]
    },
    "interface": {
        "prefabs": {},
        "reference_size": [
            960.0,
            720.0
        ],
        "safe_area": [
            0.0,
            0.0,
            0.0,
            0.0
        ],
        "scale": "ConstantPixel",
        "styles": {},
        "stylesheets": [],
        "theme": "Dark",
        "widgets": []
    },
    "lighting": {
        "ambient_brightness": 80.0,
        "ambient_color": "#FFFFFF",
        "ao_enabled": false,
        "illuminance": 10000.0,
        "light_color": "#FFFFFF",
        "light_direction": [
            8.0,
            16.0,
            8.0
        ],
        "ray_tracing": {
            "bounces": 3,
            "denoiser": "Auto",
            "enabled": false,
            "gi_distance": 50.0,
            "mode": "Hybrid",
            "paths": 1,
            "samples": 8
        },
        "shadow_bias": 0.019999999552965164,
        "shadow_map_size": 2048,
        "shadows": {
            "cascade_blend": 0.20000000298023224,
            "cascades": 4,
            "contact": false,
            "contact_length": 0.30000001192092896,
            "contact_thickness": 0.10000000149011612,
            "distance": 150.0,
            "fade": 0.10000000149011612,
            "filter": "Gaussian",
            "first_cascade": 5.0,
            "normal_bias": 1.7999999523162842,
            "sun_size": 0.0
        },
        "sun_cookie": "",
        "sun_cookie_size": 20.0
    },
    "lightning": {
        "color": "#D8E4FF",
        "decay": 0.3499999940395355,
        "flash_height": 60.0,
        "intensity": 20000000.0,
        "range": 2000.0,
        "rate": 6.0,
        "region_max": [
            200.0,
            0.0,
            200.0
        ],
        "region_min": [
            -200.0,
            0.0,
            -200.0
        ],
        "seed": 1,
        "sky_pulse": 6.0,
        "storm": false,
        "thunder": true,
        "thunder_sound": "",
        "thunder_volume": 80.0
    },
    "mode": "TwoD",
    "navigation": {
        "areas": [],
        "links": []
    },
    "post": {
        "ao": {
            "intensity": 1.0,
            "radius": 0.7285000085830688
        },
        "auto_exposure": {
            "compensation": 0.0,
            "enabled": false,
            "max_ev": 16.0,
            "metering": "Spot",
            "min_ev": 2.0,
            "speed_down": 1.0,
            "speed_up": 3.0
        },
        "bloom_dirt": "",
        "bloom_dirt_intensity": 0.0,
        "bloom_enabled": false,
        "bloom_intensity": 0.15000000596046448,
        "bloom_knee": 0.5,
        "bloom_scatter": 0.699999988079071,
        "bloom_threshold": 1.0,
        "chromatic_aberration": 0.0,
        "depth_of_field": {
            "bokeh": "Hexagonal",
            "enabled": false,
            "f_stops": 2.799999952316284,
            "far_limit": 0.0,
            "focus_distance": 10.0,
            "max_blur": 32.0,
            "near": true,
            "target": ""
        },
        "exposure_ev": 9.699999809265137,
        "grading": {
            "contrast": 1.0,
            "gain": [
                1.0,
                1.0,
                1.0
            ],
            "gamma": [
                1.0,
                1.0,
                1.0
            ],
            "lift": [
                0.0,
                0.0,
                0.0
            ],
            "lut": "",
            "lut_contribution": 1.0,
            "saturation": 1.0,
            "temperature": 0.0,
            "tint": 0.0
        },
        "grain": {
            "intensity": 0.0,
            "response": 0.800000011920929,
            "size": 1.5
        },
        "motion_blur": {
            "enabled": false,
            "samples": 4,
            "shutter_angle": 180.0
        },
        "sharpen": 0.0,
        "ssr": {
            "enabled": false,
            "roughness_cutoff": 0.4000000059604645,
            "thickness": 0.25
        },
        "tone": {
            "shoulder": 0.0,
            "toe": 0.0
        },
        "tonemapping": "TonyMcMapface",
        "vignette_strength": 0.0
    },
    "quality": {
        "auto_drop": false,
        "dlss_mode": "Auto",
        "dynamic_resolution": false,
        "min_scale": 0.5,
        "over_budget_frames": 120,
        "preset": "High",
        "resolution_scale": 1.0,
        "sharpness": 0.0,
        "target_ms": 16.66699981689453,
        "upscaler": "Spatial"
    },
    "sky": {
        "ambient_dimmer": 1.0,
        "aurora": {
            "altitude": 100.0,
            "bottom_color": "#38FF8A",
            "brightness": 8.0,
            "enabled": false,
            "height": 150.0,
            "horizon_glow": 0.30000001192092896,
            "kp": 4.0,
            "layers": 2,
            "pole_azimuth": 0.0,
            "ray_scale": 1.5,
            "speed": 1.0,
            "top_color": "#A64DFF",
            "width": 60.0
        },
        "background": true,
        "exposure": 0.0,
        "gradient": {
            "bottom": "#3A3F47",
            "brightness": 1000.0,
            "dither": true,
            "horizon_offset": 0.0,
            "middle": "#A9CBE8",
            "softness": 0.4000000059604645,
            "top": "#2F6BC4",
            "warm_color": "#FF9A50",
            "warmth": 0.5
        },
        "hdri": {
            "blur": 0.0,
            "brightness": 1000.0,
            "path": "",
            "rotation": 0.0,
            "seam_fix": 0.0,
            "tilt": 0.0,
            "tint": "#FFFFFF"
        },
        "kind": "Flat",
        "lighting": true,
        "physical": {
            "atmosphere_height": 100.0,
            "ground_albedo": "#5A5A5A",
            "horizon_curve": 1.0,
            "limb_darkening": 0.6000000238418579,
            "mie": 3.996000051498413,
            "mie_g": 0.800000011920929,
            "mie_height": 1.2000000476837158,
            "moon": false,
            "moon_azimuth": 300.0,
            "moon_brightness": 2500.0,
            "moon_color": "#C9D6FF",
            "moon_elevation": 30.0,
            "moon_halo": 0.029999999329447746,
            "moon_halo_power": 1000.0,
            "moon_light": true,
            "moon_lux": 0.25,
            "moon_phase": 0.5,
            "moon_shadows": false,
            "moon_size": 0.5199999809265137,
            "night_brightness": 1.0,
            "night_color": "#1A2B4D",
            "night_ramp": [
                2.0,
                -12.0
            ],
            "ozone": [
                0.6499999761581421,
                1.88100004196167,
                0.08500000089406967
            ],
            "planet_radius": 6360.0,
            "rayleigh": [
                5.802000045776367,
                13.557999610900879,
                33.099998474121094
            ],
            "rayleigh_height": 8.0,
            "sun_intensity": 1.0,
            "sun_size": 0.5299999713897705,
            "tint_sun": true
        },
        "reflections": true,
        "stars": {
            "brightness": 60.0,
            "color_variation": 0.5,
            "density": 0.3499999940395355,
            "enabled": false,
            "horizon_fade": 8.0,
            "magnitude_slope": 3.0,
            "milky_way": "",
            "milky_way_brightness": 2.0,
            "milky_way_rotation": 0.0,
            "milky_way_tilt": 60.0,
            "sun_fade": [
                -2.0,
                -14.0
            ],
            "twinkle": 0.30000001192092896,
            "twinkle_speed": 1.5
        },
        "sun": {
            "azimuth": 135.0,
            "day_of_year": 172,
            "elevation": 54.70000076293945,
            "latitude": 40.0,
            "longitude": 0.0,
            "mode": "Light",
            "time_of_day": 12.0,
            "utc_offset": 0.0
        }
    },
    "sound": {
        "master_volume": 1.0,
        "music_volume": 1.0,
        "sfx_volume": 1.0
    },
    "speech_bubble": {
        "background": "#FFFFFF",
        "border": "#B8C0CC",
        "font_asset": null,
        "font_size": 18.0,
        "max_width": 260.0,
        "offset": [
            0.0,
            -12.0
        ],
        "padding": [
            12.0,
            8.0
        ],
        "text": "#172033"
    },
    "surface": {
        "snow": 0.0,
        "wetness": 0.0
    },
    "vfx": {
        "budget": 200000,
        "cpu_only": false
    },
    "wind": {
        "clouds": {
            "advection": [
                0.0,
                0.0,
                0.0
            ],
            "altitude": 1500.0,
            "erosion": [
                0.0,
                0.5,
                0.0
            ],
            "follow": 1.0,
            "layer_scroll": 1.0,
            "seed": 1,
            "time_lapse": 1.0
        },
        "direction": 45.0,
        "ground": 0.0,
        "gust": 0.0,
        "gust_frequency": 0.20000000298023224,
        "profile": true,
        "reference_height": 10.0,
        "roughness": 0.10000000149011612,
        "seed": 1,
        "speed": 0.0,
        "storm": 0.0,
        "veer": 15.0
    }
}; }
