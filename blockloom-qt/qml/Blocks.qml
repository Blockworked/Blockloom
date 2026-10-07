pragma Singleton
import QtQuick
import com.blockworked.Blockstitch 1.0

// Blockloom's block vocabulary, as data: every block's row, the fresh copy the
// palette offers, the sensing reporters, and the groups the sidebar shows.
// Registered with blockstitch's BlockRegistry once, at startup. Adding a
// block means a row here, a prefab, and a palette group entry.
QtObject {
    id: blocks

    // The backend snapshot, and the actor whose canvas is open. Main keeps
    // both current; option lists read them.
    property var appState: null
    readonly property var project: appState && appState.project ? appState.project : null
    readonly property var actor: {
        if (!project || !appState.selected_actor) return null;
        for (const a of project.actors) if (a.id === appState.selected_actor) return a;
        return null;
    }
    readonly property string mode: project ? project.world.mode : "TwoD"
    // Option lists are functions of the state, which BlockRegistry can't
    // watch, so its revision goes up when anything they read changes. Only
    // then: a bump re-evaluates every dropdown on the canvas and the palette.
    readonly property string optionSource: {
        if (!project) return "";
        const names = xs => (xs || []).map(x => x.name);
        return JSON.stringify([mode, names(project.actors), names(project.world.input ? project.world.input.actions : []),
            names(project.globals), names(project.global_lists), names(project.global_dicts),
            actor ? [names(actor.variables), names(actor.lists), names(actor.dicts),
                     actor.components.map(c => [componentName(c), (c.fields || []).map(f => f.name)])] : null]);
    }
    onOptionSourceChanged: BlockRegistry.revision++

    // ─── Fixed option lists (constants.ts) ─────────────────────────────────
    function opts(names) { return names.map(n => ({ value: n, label: n })); }
    readonly property var keyOptions: opts(["space","up arrow","down arrow","left arrow","right arrow","enter","escape","shift","control","alt","tab","backspace","back"]
        .concat("abcdefghijklmnopqrstuvwxyz".split("")).concat("0123456789".split("")))
    // The atmosphere slot's readings, spelled as `AtmosphereSense::field` takes them.
    readonly property var atmosphereOptions: opts(["sun x","sun y","sun z","sun brightness","wind x","wind y","wind z","wind speed","wind gust","wind direction","storm","fog density","cloud cover","rain","snow","wetness","temperature","exposure","luminance","hdr","peak brightness","ray tracing","ray tracing available","aurora","lightning","time of day","sun elevation"])
    readonly property var particleEventOptions: [{value:"Spawn",label:"spawn"},{value:"Die",label:"die"},{value:"Collide",label:"collide"}]
    readonly property var axisOptions: [{value:"X",label:"x"},{value:"Y",label:"y"},{value:"Z",label:"z"}]
    readonly property var bodyOptions: [{value:"None",label:"none (blocks only)"},{value:"Static",label:"static"},{value:"Dynamic",label:"dynamic"},{value:"Kinematic",label:"kinematic"}]
    readonly property var triggerOptions: [{value:"false",label:"solid"},{value:"true",label:"a trigger"}]
    readonly property var layerOptions: opts(["1","2","3","4","5","6","7","8"])
    readonly property var cameraViewOptions: [{value:"Follow",label:"follow"},{value:"FirstPerson",label:"first person"},{value:"ThirdPerson",label:"third person"}]
    readonly property var visibleOptions: [{value:"true",label:"show"},{value:"false",label:"hide"}]
    readonly property var mouseLockOptions: [{value:"true",label:"lock"},{value:"false",label:"unlock"}]
    readonly property var uiAnchorOptions: [{value:"TopLeft",label:"top left"},{value:"Top",label:"top"},{value:"TopRight",label:"top right"},{value:"Left",label:"left"},{value:"Center",label:"centre"},{value:"Right",label:"right"},{value:"BottomLeft",label:"bottom left"},{value:"Bottom",label:"bottom"},{value:"BottomRight",label:"bottom right"}]
    readonly property var uiPropOptions: [{value:"Text",label:"text"},{value:"TextColor",label:"text color"},{value:"TextSize",label:"text size"},{value:"Background",label:"background"},{value:"Width",label:"width"},{value:"Height",label:"height"},{value:"Visible",label:"visible"},{value:"CornerRadius",label:"corner radius"},{value:"Padding",label:"padding"},{value:"Modal",label:"modal"},{value:"Min",label:"min"},{value:"Max",label:"max"},{value:"Value",label:"value"},{value:"Step",label:"step"},{value:"Allow",label:"allow"},{value:"MaxLength",label:"max length"},{value:"Layout",label:"Layout"},{value:"Style",label:"Style"},{value:"Bind",label:"Bind"},{value:"Items",label:"Items"},{value:"Scroll",label:"Scroll"},{value:"SelectedIndex",label:"SelectedIndex"},{value:"Theme",label:"Theme"},{value:"Enabled",label:"Enabled"},{value:"Tooltip",label:"Tooltip"},{value:"WorldActor",label:"WorldActor"},{value:"Transition",label:"Transition"}]
    readonly property var modalOptions: [{value:"false",label:"floating"},{value:"true",label:"modal"}]
    readonly property var onOffOptions: [{value:"true",label:"on"},{value:"false",label:"off"}]
    readonly property var enableOptions: [{value:"true",label:"enable"},{value:"false",label:"disable"}]
    readonly property var uiThemeOptions: [{value:"Dark",label:"dark"},{value:"Light",label:"light"},{value:"HighContrast",label:"high contrast"}]
    readonly property var waterOptions: [{value:"Level",label:"level"},{value:"Chop",label:"chop"},{value:"Foam",label:"foam"}]
    readonly property var rayHitsOptions: [{value:"Nearest",label:"nearest"},{value:"Every",label:"every hit"}]
    readonly property var triggerPolicyOptions: [{value:"UseGlobal",label:"project default"},{value:"Ignore",label:"ignore triggers"},{value:"Include",label:"hit triggers"}]
    readonly property var hitNumberOptions: opts(["count","overflowed","tick","x","y","z","normal x","normal y","normal z","distance","fraction","started inside","is trigger","part"])
    readonly property var hitTextOptions: opts(["actor","actor id","body","collider","error"])
    readonly property var controllerNumberOptions: opts(["grounded","flags","sides","above","below","moved x","moved y","moved z","asked x","asked y","asked z","velocity x","velocity y","velocity z","fall speed","recovered","stepped","skipped","hit count","hit x","hit y","hit z","normal x","normal y","normal z","hit length","radius","height","tick"])
    readonly property var controllerTextOptions: opts(["actor","body","collider","error"])
    readonly property var moveModeOptions: [{value:"Move",label:"by"},{value:"Simple",label:"at speed"}]
    readonly property var controllerPropertyOptions: [{value:"Enabled",label:"enabled"},{value:"Radius",label:"radius"},{value:"Height",label:"height"},{value:"SlopeLimit",label:"slope limit"},{value:"StepOffset",label:"step offset"},{value:"SkinWidth",label:"skin width"},{value:"MinMoveDistance",label:"minimum move"},{value:"DetectCollisions",label:"detect collisions"},{value:"OverlapRecovery",label:"overlap recovery"}]
    readonly property var motorNumberOptions: opts(["grounded","rising","falling","landed","jumped","left ground","hit head","changed stance","crouching","speed","desired speed","vertical speed","can jump","jumps used","jumps left","slope","ground normal x","ground normal y","ground normal z","knockback","walk speed","jump height","enabled"])
    readonly property var motorTextOptions: opts(["state","support","owner","warning"])
    readonly property var jointActionOptions: [{value:"MotorSpeed",label:"turn motor at speed"},{value:"MotorTarget",label:"drive motor to"},{value:"MotorForce",label:"limit motor force to"},{value:"MotorOff",label:"stop motor"},{value:"Stiffness",label:"set stiffness to"},{value:"Damping",label:"set damping to"},{value:"Enable",label:"switch on"},{value:"Disable",label:"switch off"},{value:"Break",label:"break"}]
    readonly property var jointNumberOptions: opts(["angle","position","speed","force","torque","broken","enabled","count"])
    readonly property var motorActionOptions: [{value:"Intent",label:"steer"},{value:"Jump",label:"jump"},{value:"JumpRelease",label:"let go of jump"},{value:"SprintOn",label:"start sprinting"},{value:"SprintOff",label:"stop sprinting"},{value:"CrouchOn",label:"start crouching"},{value:"CrouchOff",label:"stop crouching"},{value:"Push",label:"push"},{value:"Stop",label:"stop"}]
    readonly property var motorPropertyOptions: [{value:"Enabled",label:"enabled"},{value:"WalkSpeed",label:"walk speed"},{value:"SprintSpeed",label:"sprint speed"},{value:"CrouchSpeed",label:"crouch speed"},{value:"Acceleration",label:"acceleration"},{value:"Braking",label:"braking"},{value:"AirAcceleration",label:"air acceleration"},{value:"AirControl",label:"air control"},{value:"TurnSpeed",label:"turn speed"},{value:"GravityScale",label:"gravity scale"},{value:"TerminalSpeed",label:"terminal speed"},{value:"JumpHeight",label:"jump height"},{value:"MaxJumps",label:"max jumps"},{value:"CoyoteTime",label:"coyote time"},{value:"JumpBuffer",label:"jump buffer"},{value:"SlideOnSteep",label:"slide on steep"}]
    readonly property var forceModeOptions: [{value:"Force",label:"force (N)"},{value:"Acceleration",label:"acceleration"},{value:"Impulse",label:"impulse"},{value:"VelocityChange",label:"velocity change"}]
    readonly property var contactPhaseOptions: [{value:"Enter",label:"start touching"},{value:"Stay",label:"keep touching"},{value:"Exit",label:"stop touching"}]
    readonly property var contactScopeOptions: [{value:"Any",label:"any touch"},{value:"Collision",label:"solid"},{value:"Trigger",label:"trigger"}]
    readonly property var parallaxAxisOptions: [{value:"Both",label:"both axes"},{value:"X",label:"x"},{value:"Y",label:"y"}]
    readonly property var windOptions: [{value:"Direction",label:"direction"},{value:"Speed",label:"speed"},{value:"Gust",label:"gust"},{value:"Storm",label:"storm"}]
    readonly property var cloudLayerOptions: [{value:"Coverage",label:"coverage"},{value:"Opacity",label:"opacity"},{value:"Contrast",label:"contrast"},{value:"Altitude",label:"altitude"},{value:"Spin",label:"spin"}]
    readonly property var cloudOptions: [{value:"Coverage",label:"coverage"},{value:"Density",label:"density"},{value:"Type",label:"type"}]
    readonly property var precipitationOptions: [{value:"Rain",label:"rain"},{value:"Snow",label:"snow"}]
    readonly property var soundBusOptions: [{value:"Master",label:"master"},{value:"Music",label:"music"},{value:"Sfx",label:"sound effects"}]
    readonly property var soundLoopOptions: [{value:"false",label:"once"},{value:"true",label:"loop"}]
    readonly property var easingOptions: opts(["Linear","EaseIn","EaseOut","EaseInOut","Bounce","Elastic"])
    readonly property var loopModeOptions: opts(["Once","Loop","PingPong"])
    readonly property var gamepadButtonOptions: opts(["South","East","North","West","C","Z","LeftTrigger","LeftTrigger2","RightTrigger","RightTrigger2","Select","Start","Mode","LeftThumb","RightThumb","DPadUp","DPadDown","DPadLeft","DPadRight"])
    readonly property var gamepadAxisOptions: opts(["LeftStickX","LeftStickY","LeftZ","RightStickX","RightStickY","RightZ"])
    readonly property var mouseButtonOptions: opts(["left","right","middle"])

    // ─── Names the open actor can see (types.ts) ───────────────────────────
    // Its own first, then the project's shared ones it doesn't shadow.
    function scoped(own, shared) {
        const mine = (own || []).map(x => x.name).sort((a, b) => a.localeCompare(b));
        const theirs = (shared || []).map(x => x.name).filter(n => mine.indexOf(n) < 0).sort((a, b) => a.localeCompare(b));
        return mine.concat(theirs);
    }
    function variableNames() { return scoped(actor ? actor.variables : [], project ? project.globals : []); }
    function listNames() { return scoped(actor ? actor.lists : [], project ? project.global_lists : []); }
    function dictNames() { return scoped(actor ? actor.dicts : [], project ? project.global_dicts : []); }
    // Lists/dicts with the actor's own shadowing a shared one of the same name.
    function visibleCollections(own, shared) {
        const mine = own || [];
        return mine.concat((shared || []).filter(s => !mine.some(o => o.name === s.name)));
    }
    function componentName(c) { return c.component === "Custom" ? c.name : c.component; }
    function customComponents() { return (actor ? actor.components : []).filter(c => c.component === "Custom"); }

    function actorOptions(withMouse) {
        const list = (project ? project.actors : []).map(a => ({ value: a.name, label: a.name }));
        return withMouse ? list.concat([{ value: "mouse", label: "the mouse" }]) : list;
    }
    function actionOptions() {
        const actions = project && project.world.input ? project.world.input.actions : [];
        return actions.length ? actions.map(a => ({ value: a.name, label: a.name })) : [{ value: "Jump", label: "Jump" }];
    }
    function attachableOptions() {
        const held = (actor ? actor.components : []).map(componentName);
        const extra = ["Look","Render","Body","Joint","Brain","Camera","Material","Emitter","Trail","Light","Animation","Volume","Probe","Persist"].filter(n => held.indexOf(n) < 0);
        return opts(held.filter(n => n !== "Place" && n !== "Script").concat(extra));
    }
    function detachableOptions() { return opts((actor ? actor.components : []).map(componentName).filter(n => n !== "Place")); }
    function componentFieldOptions(ins) {
        const chosen = ins && typeof ins.component === "string" ? ins.component : "";
        const c = customComponents().find(x => x.name === chosen);
        return opts(c ? c.fields.map(f => f.name) : []);
    }

    // ─── Row pieces (blockFields.ts) ───────────────────────────────────────
    function is3d() { return blocks.mode === "ThreeD"; }
    function lb(text, only3d) { return only3d ? { kind: "label", text: text, when: is3d } : { kind: "label", text: text }; }
    function slot(field, key, extra) { return Object.assign({ kind: "value", field: field, key: key }, extra || {}); }
    function dd(key, options, extra) { return Object.assign({ kind: "dropdown", key: key, options: options }, extra || {}); }
    function field(key, placeholder) { return { kind: "text", key: key, placeholder: placeholder }; }
    function flag(key, options) {
        return dd(key, options, { encode: c => c === "true", decode: v => (v === true ? "true" : "false") });
    }
    function flagDefaultTrue(key, options) {
        return dd(key, options, { encode: c => c === "true", decode: v => (v === false ? "false" : "true") });
    }
    function vector(prefix, fields, keys) {
        return [lb(prefix + " x:"), slot(fields[0], keys[0]), lb("y:"), slot(fields[1], keys[1]), lb("z:", true), slot(fields[2], keys[2], { when: is3d })];
    }
    function placement() {
        return [lb("at"), dd("anchor", uiAnchorOptions), lb("+ x:"), slot("UiX", "x"), lb("y:"), slot("UiY", "y"),
                lb("size"), slot("UiWidth", "width"), lb("x"), slot("UiHeight", "height"), lb("in"), slot("UiParent", "parent")];
    }
    function showRow(verb, contentKey, extra) {
        return [lb(verb), slot("UiId", "element")].concat(contentKey ? [slot("UiContent", contentKey)] : []).concat(extra || []).concat(placement());
    }
    function positionAxes() { return is3d() ? axisOptions : axisOptions.filter(a => a.value !== "Z"); }
    function rotationAxes() { return is3d() ? axisOptions : axisOptions.filter(a => a.value === "Z"); }
    function varD() { return dd("name", () => opts(variableNames()), { placeholder: "variable" }); }
    function listD() { return dd("name", () => opts(listNames()), { placeholder: "list" }); }
    function dictD() { return dd("name", () => opts(dictNames()), { placeholder: "dict" }); }

    readonly property var icons: ({
        Fracture:"hammer", Splash:"droplets", PuffSmoke:"cloud", SpawnDecal:"stamp", FadeDecals:"eraser",
        WhenStarted:"flag", WhenKeyPressed:"keyboard", WhenActionPressed:"gamepad-2", WhenTouched:"pointer", WhenClicked:"mouse-pointer-click",
        WhenQualityDrops:"gauge", SetRenderSetting:"gauge", WhenCollision:"crosshair", WhenMessage:"radio", WhenCloned:"copy", WhenParticles:"sparkles", WhenAnimationMarker:"flag", WhenEnterRoom:"square-dashed", WhenUiEvent:"square-mouse-pointer", WhenWeather:"cloud", WhenUiClicked:"square-mouse-pointer", WhenUiChanged:"sliders-horizontal",
        BlockHeader:"blocks", Move:"arrow-right", GoTo:"move", NavigateTo:"navigation", ChangePosition:"move-3d", Glide:"wind", TweenScale:"maximize", TweenRotation:"rotate-cw", TweenColor:"palette", StopTweens:"square", PlayAnimation:"play", StopAnimation:"square", SetAnimationSpeed:"gauge", FireAnimationTrigger:"zap", SetRigSlot:"git-branch", SetSlotTint:"palette", SetIkTarget:"crosshair", SetSpriteDial:"sliders-horizontal", Turn:"rotate-cw",
        SetRotation:"rotate-cw", PointTowards:"target", SetScale:"maximize", SetBody:"boxes", SetTrigger:"ghost", SetCollisionLayer:"layers",
        SetCollisionMask:"filter", ApplyImpulse:"zap", AddForce:"arrow-right", AddTorque:"rotate-cw", ControllerMove:"footprints", SetController:"sliders-horizontal", MotorAct:"person-standing", SetMotor:"gauge", JointAct:"link", CastRay:"crosshair", CastBall:"circle-dot", OverlapBall:"search", FindClosest:"target", SetVelocity:"trending-up", SetGravity:"cloud", SetDensity:"weight", SetMass:"weight",
        Say:"message-square", SetVisible:"eye", SetColor:"palette", SetExposure:"sun", SetLightIntensity:"zap", SetEmissiveStrength:"sparkles", SetHdrOutput:"monitor", SetPeakBrightness:"sun", EnableVolume:"box", SetVolumeWeight:"weight", CaptureProbes:"aperture", SetShadowDistance:"sun-dim", SetLightShadows:"lamp", SetRayTracing:"sparkles", SetGiBounces:"repeat", SetGiSamples:"layers", SetFogDensity:"cloud", SetAurora:"moon", StrikeLightning:"zap", SetLightningRate:"wind", SetWind:"wind", SetCloudDrift:"cloud", SetClouds:"cloud", SetCloudLayer:"layers", SetWater:"wind", SetTimeOfDay:"clock", AdvanceTime:"clock", SetPrecipitation:"cloud-rain", BlendWeather:"cloud", PaintTile:"palette", SetParallax:"layers", BurstParticles:"sparkles", SetEmitterDial:"sliders-horizontal", SetTrailEnabled:"wind", SetEmitterPlaying:"play", PlaySound:"volume-2", PlaySoundAt:"map-pin", StopSound:"square",
        SetSoundVolume:"volume-1", SetSoundPitch:"music", SetBusVolume:"sliders-horizontal", SetComponentField:"panels-top-left",
        SetCameraView:"camera", SetCameraPitch:"video", SetCameraFov:"video", AttachComponent:"plus", DetachComponent:"x", SetParent:"link",
        CreateClone:"copy", CreateActor:"sparkles", DeleteActor:"trash-2", Wait:"clock", WaitUntil:"hand", If:"git-branch", IfElse:"git-fork",
        Repeat:"repeat", Forever:"infinity", While:"repeat", EscapeLoop:"log-out", ContinueLoop:"skip-forward", Broadcast:"radio", StopAll:"octagon",
        SwitchScene:"layers", WhenSceneStarts:"play", WhenSceneEnds:"square", PlayCutscene:"video", SkipCutscene:"skip-forward", CameraShake:"zap", SetTimeScale:"timer", Hitstop:"square", SetLetterbox:"eye", FadeScreen:"square", WhenCutsceneSignal:"flag", WhenCutsceneEnds:"square",
        SetMouseLocked:"lock", RumbleGamepad:"vibrate", BindAction:"keyboard", ClearActionBindings:"eraser", ShowPanel:"layout-panel-top",
        ShowLabel:"type", ShowButton:"square-mouse-pointer", ShowImage:"image", ShowInput:"text-cursor-input", ShowSlider:"sliders-horizontal",
        ShowWidget:"layout-grid", ShowToggle:"toggle-left", ShowList:"list-checks", SetUiTheme:"palette", BindUi:"list-checks", SetUiItems:"list-checks", ScrollUi:"list-checks", SetElementTheme:"list-checks", SetUiProp:"list-checks", HideElement:"eye", HideAllUi:"eye",
        DeleteElement:"trash-2", FocusElement:"text-select", ClearFocus:"text-select", PauseGame:"pause", ResumeGame:"play", SetVariable:"asterisk",
        ChangeVariable:"trending-up", SaveVariable:"save", ClearSavedVariable:"trash-2", AddToList:"plus", DeleteOfList:"trash-2",
        DeleteAllOfList:"trash-2", ShiftList:"arrow-left", InsertIntoList:"plus", ReplaceItemOfList:"repeat", ReverseList:"rotate-cw",
        LoadJsonIntoList:"braces", SetDictValue:"book-plus", DeleteDictKey:"trash-2", DeleteAllOfDict:"trash-2", LoadJsonIntoDict:"braces",
        CallBlock:"blocks", Return:"corner-down-right", PluginBlock:"plug-zap", WhenPlugin:"plug-zap"
    })

    function buildRows() {
        const header = (head) => ({ shape: "header", head: head });
        const cap = (head) => ({ shape: "cap", head: head });
        const row = (head) => ({ head: head });
        const r = {
            // Events
            WhenStarted: header([lb("when the project starts")]),
            WhenKeyPressed: header([lb("when"), dd("key", keyOptions), lb("pressed")]),
            WhenActionPressed: header([lb("when action"), dd("action", actionOptions, { placeholder: "action" }), lb("pressed")]),
            WhenTouched: header([lb("when the screen is touched")]),
            WhenClicked: header([lb("when I am clicked")]),
            WhenCollision: header([lb("when I"), dd("phase", contactPhaseOptions), dd("with", () => [{ value: "", label: "anything" }].concat(actorOptions(false)), { placeholder: "anything" }), lb("as"), dd("scope", contactScopeOptions)]),
            WhenMessage: header([lb("when I get"), field("name", "message")]),
            WhenCloned: header([lb("when I start as a clone")]),
            WhenParticles: header([lb("when my particles"), dd("event", () => particleEventOptions)]),
            WhenUiEvent: header([lb("when"), field("element", "element id"), dd("event", () => opts(["press", "release", "hover", "leave", "drag", "scroll", "focus"]))]),
            WhenUiClicked: header([lb("when"), field("element", "element id"), lb("clicked")]),
            WhenUiChanged: header([lb("when"), field("element", "element id"), lb("changed")]),
            WhenAnimationEnds: header([lb("when animation"), field("clip", "clip name (empty for any)"), lb("ends")]),
            WhenAnimationMarker: header([lb("when animation reaches marker"), field("marker", "marker (empty for any)")]),
            WhenEnterRoom: header([lb("when I enter room"), field("room", "room (empty for any)")]),
            WhenWeather: header([lb("when weather becomes"), field("weather", "weather (empty for any)")]),
            WhenSceneStarts: header([lb("when scene starts")]),
            WhenSceneEnds: header([lb("when scene ends")]),
            WhenCutsceneSignal: header([lb("when cutscene signal"), field("signal", "signal (empty for any)")]),
            WhenCutsceneEnds: header([lb("when cutscene ends")]),
            Broadcast: row([lb("broadcast"), field("name", "message")]),
            SwitchScene: row([lb("switch scene to"), slot("SceneName", "scene"), lb("with transition"), slot("SceneTransition", "none")]),
            PlayCutscene: row([lb("play cutscene"), slot("CutsceneName", "cutscene")]),
            SkipCutscene: row([lb("skip cutscene")]),
            CameraShake: row([lb("shake camera by"), slot("ShakeAmount", "amount")]),
            SetTimeScale: row([lb("set time scale to"), slot("TimeScale", "scale")]),
            Hitstop: row([lb("hitstop"), slot("HitstopFrames", "frames"), lb("frames")]),
            SetLetterbox: row([lb("set letterbox to"), slot("LetterboxBars", "on")]),
            FadeScreen: row([lb("fade screen to"), slot("FadeColor", "color")]),
            // Motion
            Move: row([lb("move"), slot("MoveSteps", "steps"), lb("steps")]),
            GoTo: row(vector("go to", ["GoToX","GoToY","GoToZ"], ["x","y","z"])),
            NavigateTo: row(vector("navigate to", ["NavigateX","NavigateY","NavigateZ"], ["x","y","z"]).concat([lb("at speed"), slot("NavigateSpeed", "speed")])),
            ChangePosition: row([lb("change"), dd("axis", positionAxes), lb("by"), slot("ChangeByAmount", "by")]),
            Glide: row([lb("glide"), slot("GlideSeconds", "seconds"), lb("secs to x:"), slot("GlideX", "x"), lb("y:"), slot("GlideY", "y"), lb("z:", true), slot("GlideZ", "z", { when: is3d }), dd("easing", easingOptions)]),
            TweenScale: row([lb("tween size to"), slot("TweenFactor", "factor"), lb("in"), slot("TweenSeconds", "seconds"), lb("secs"), dd("easing", easingOptions)]),
            TweenRotation: row([lb("tween"), dd("axis", rotationAxes, { when: is3d }), lb("rotation to"), slot("TweenDegrees", "degrees"), lb("deg in"), slot("TweenSeconds", "seconds"), lb("secs"), dd("easing", easingOptions)]),
            TweenColor: row([lb("tween color to"), slot("TweenColor", "color"), lb("in"), slot("TweenSeconds", "seconds"), lb("secs"), dd("easing", easingOptions)]),
            StopTweens: row([lb("stop my tweens")]),
            Turn: row([lb("turn"), dd("axis", rotationAxes, { when: is3d }), lb("by"), slot("TurnDegrees", "degrees"), lb("degrees")]),
            SetRotation: row([lb("point"), dd("axis", rotationAxes, { when: is3d }), lb("in direction"), slot("RotationDegrees", "degrees")]),
            PointTowards: row([lb("point towards"), dd("target", () => actorOptions(true))]),
            SetScale: row([lb("set size to"), slot("ScaleFactor", "factor")]),
            // Physics
            SetBody: row([lb("set body to"), dd("body", bodyOptions)]),
            ApplyImpulse: row(vector("push", ["ImpulseX","ImpulseY","ImpulseZ"], ["x","y","z"])),
            AddForce: row([lb("add force"), dd("mode", forceModeOptions), lb("x:"), slot("ForceX", "x"), lb("y:"), slot("ForceY", "y"), lb("z:", true), slot("ForceZ", "z", { when: is3d })]),
            CastRay: row([lb("cast ray"), dd("hits", rayHitsOptions), dd("triggers", triggerPolicyOptions), lb("from x:"), slot("QueryFromX", "from_x"), lb("y:"), slot("QueryFromY", "from_y"), lb("z:", true), slot("QueryFromZ", "from_z", { when: is3d }), lb("to x:"), slot("QueryToX", "to_x"), lb("y:"), slot("QueryToY", "to_y"), lb("z:", true), slot("QueryToZ", "to_z", { when: is3d })]),
            CastBall: row([lb("cast ball of radius"), slot("QueryRadius", "radius"), dd("triggers", triggerPolicyOptions), lb("from x:"), slot("QueryFromX", "from_x"), lb("y:"), slot("QueryFromY", "from_y"), lb("z:", true), slot("QueryFromZ", "from_z", { when: is3d }), lb("to x:"), slot("QueryToX", "to_x"), lb("y:"), slot("QueryToY", "to_y"), lb("z:", true), slot("QueryToZ", "to_z", { when: is3d })]),
            OverlapBall: row([lb("find everything in ball of radius"), slot("QueryRadius", "radius"), dd("triggers", triggerPolicyOptions), lb("at x:"), slot("QueryFromX", "x"), lb("y:"), slot("QueryFromY", "y"), lb("z:", true), slot("QueryFromZ", "z", { when: is3d })]),
            FindClosest: row([lb("find closest within"), slot("QueryRadius", "range"), dd("triggers", triggerPolicyOptions), lb("of x:"), slot("QueryFromX", "x"), lb("y:"), slot("QueryFromY", "y"), lb("z:", true), slot("QueryFromZ", "z", { when: is3d })]),
            ControllerMove: row([lb("move controller"), dd("mode", moveModeOptions), lb("x:"), slot("ControllerX", "x"), lb("y:"), slot("ControllerY", "y"), lb("z:", true), slot("ControllerZ", "z", { when: is3d })]),
            MotorAct: row([lb("motor"), dd("action", motorActionOptions), lb("x:"), slot("ControllerX", "x"), lb("y:"), slot("ControllerY", "y"), lb("z:", true), slot("ControllerZ", "z", { when: is3d })]),
            JointAct: row([lb("joint"), field("joint", "name"), dd("action", jointActionOptions), lb("value"), slot("JointValue", "value")]),
            SetMotor: row([lb("set motor"), dd("property", motorPropertyOptions), lb("to"), slot("ControllerValue", "value")]),
            SetController: row([lb("set controller"), dd("property", controllerPropertyOptions), lb("to"), slot("ControllerValue", "value")]),
            AddTorque: row([lb("add torque"), dd("mode", forceModeOptions), lb("x:", true), slot("ForceX", "x", { when: is3d }), lb("y:", true), slot("ForceY", "y", { when: is3d }), lb("z:"), slot("ForceZ", "z")]),
            SetVelocity: row(vector("set velocity", ["VelocityX","VelocityY","VelocityZ"], ["x","y","z"])),
            SetGravity: row(vector("set world gravity", ["GravityX","GravityY","GravityZ"], ["x","y","z"])),
            SetDensity: row([lb("set density to"), slot("Density", "density")]),
            SetMass: row([lb("set mass to"), slot("Mass", "mass")]),
            SetTrigger: row([lb("make me"), flag("trigger", triggerOptions)]),
            SetCollisionLayer: row([lb("set my collision layer to"), slot("TriggerLayer", "layer")]),
            SetCollisionMask: row([lb("set my collision mask to"), slot("TriggerMask", "mask")]),
            // Looks
            Say: row([lb("say"), slot("SayText", "text")]),
            SetVisible: row([flagDefaultTrue("visible", visibleOptions), lb("myself")]),
            SetColor: row([lb("set color to"), slot("ColorText", "color")]),
            WhenQualityDrops: header([lb("when quality drops")]),
            SetRenderSetting: row([lb("set"), dd("setting", () => ["Quality", "ResolutionScale", "Upscaler", "DlssMode"].map(v => ({ value: v, label: ({Quality:"quality", ResolutionScale:"resolution scale", Upscaler:"upscaler", DlssMode:"DLSS mode"})[v] }))), lb("to"), slot("RenderSettingValue", "value")]),
            SetExposure: row([lb("set exposure to"), slot("ExposureEv", "ev"), lb("EV")]),
            SetLightIntensity: row([lb("set my light to"), slot("LightIntensity", "intensity"), lb("lumens")]),
            SetEmissiveStrength: row([lb("set my glow to"), slot("EmissiveStrength", "strength")]),
            SetHdrOutput: row([lb("turn HDR output"), flagDefaultTrue("enabled", onOffOptions)]),
            SetPeakBrightness: row([lb("set peak brightness to"), slot("PeakNits", "nits"), lb("nits")]),
            EnableVolume: row([flagDefaultTrue("enabled", enableOptions), lb("volume"), slot("VolumeTarget", "volume")]),
            SetVolumeWeight: row([lb("set weight of volume"), slot("VolumeTarget", "volume"), lb("to"), slot("VolumeWeight", "weight")]),
            CaptureProbes: row([lb("capture probes")]),
            SetShadowDistance: row([lb("set shadow distance to"), slot("ShadowDistance", "distance"), lb("m")]),
            SetLightShadows: row([lb("turn my light's shadows"), flagDefaultTrue("enabled", onOffOptions)]),
            SetRayTracing: row([lb("turn ray tracing"), flagDefaultTrue("enabled", onOffOptions)]),
            SetGiBounces: row([lb("set GI bounces to"), slot("GiBounces", "bounces")]),
            SetGiSamples: row([lb("set GI samples to"), slot("GiSamples", "samples")]),
            SetFogDensity: row([lb("set fog density to"), slot("FogDensity", "density"), lb("per m")]),
            SetAurora: row([lb("set aurora to KP"), slot("AuroraKp", "kp")]),
            StrikeLightning: row(vector("strike lightning at", ["LightningX","LightningY","LightningZ"], ["x","y","z"])),
            Fracture: row([lb("fracture"), slot("FractureTarget", "target")]),
            Splash: row(vector("splash at", ["FxX","FxY","FxZ"], ["x","y","z"]).concat([lb("radius"), slot("FxRadius", "radius"), lb("strength"), slot("FxStrength", "strength")])),
            PuffSmoke: row(vector("puff smoke at", ["FxX","FxY","FxZ"], ["x","y","z"]).concat([lb("radius"), slot("FxRadius", "radius"), lb("density"), slot("FxStrength", "strength")])),
            SpawnDecal: row([lb("spawn decal"), dd("preset", () => opts(["Blood","Footprint","FreshScorch"]))].concat(
                vector("at", ["DecalX","DecalY","DecalZ"], ["x","y","z"]),
                vector("normal", ["DecalNormalX","DecalNormalY","DecalNormalZ"], ["nx","ny","nz"]),
                [lb("size"), slot("DecalSize", "size"), lb("life"), slot("DecalLifetime", "lifetime"), lb("fade"), slot("DecalFade", "fade")])),
            FadeDecals: row(vector("fade decals at", ["DecalX","DecalY","DecalZ"], ["x","y","z"]).concat(
                [lb("in radius"), slot("DecalRadius", "radius"), lb("over"), slot("DecalSeconds", "seconds"), lb("seconds")])),
            SetLightningRate: row([lb("set lightning storm to"), slot("LightningRate", "rate"), lb("strikes a minute")]),
            SetWind: row([lb("set wind"), dd("property", windOptions), lb("to"), slot("WindValue", "value")]),
            SetClouds: row([lb("set clouds"), dd("property", cloudOptions), lb("to"), slot("CloudValue", "value")]),
            SetTimeOfDay: row([lb("set time of day to"), slot("TimeOfDay", "hours")]),
            AdvanceTime: row([lb("advance time by"), slot("AdvanceHours", "hours"), lb("hours")]),
            SetPrecipitation: row([lb("set"), dd("property", precipitationOptions), lb("to"), slot("PrecipitationValue", "value")]),
            BlendWeather: row([lb("blend weather to"), slot("BlendWeatherName", "Clear"), lb("over"), slot("BlendWeatherSeconds", "seconds"), lb("seconds")]),
            SetWater: row([lb("set water"), dd("property", waterOptions), lb("to"), slot("WaterValue", "value")]),
            PaintTile: row([lb("paint tile"), slot("TileIndex", "tile"), lb("at x"), slot("TileX", "x"), lb("y"), slot("TileY", "y"), lb("z", true), slot("TileZ", "z", { when: is3d }), lb("of map"), slot("TileMap", "map")]),
            SetParallax: row([lb("set parallax of layer"), slot("ParallaxLayer", "layer"), dd("axis", parallaxAxisOptions), lb("to"), slot("ParallaxValue", "value")]),
            SetCloudLayer: row([lb("set cloud layer"), slot("CloudLayer", "layer"), dd("property", cloudLayerOptions), lb("to"), slot("CloudLayerValue", "value")]),
            SetCloudDrift: row(vector("set cloud drift to", ["CloudDriftX","CloudDriftY","CloudDriftZ"], ["x","y","z"])),
            BurstParticles: row([lb("burst"), slot("ParticleCount", "count"), lb("particles")]),
            SetEmitterDial: row([lb("set emitter"), dd("dial", () => opts(["Rate","Lifetime","Speed","Spread","Gravity","SizeStart","SizeEnd","Max"])), lb("to"), slot("EmitterValue", "value")]),
            SetTrailEnabled: row([flagDefaultTrue("enabled", visibleOptions), lb("my trail")]),
            SetEmitterPlaying: row([flagDefaultTrue("playing", [{ value: "true", label: "start" }, { value: "false", label: "stop" }]), lb("my particles")]),
            PlayAnimation: row([lb("play animation"), slot("AnimClip", "clip"), lb("at speed"), slot("AnimSpeed", "speed")]),
            StopAnimation: row([lb("stop my animation")]),
            SetAnimationSpeed: row([lb("set animation speed to"), slot("AnimSpeed", "speed")]),
            FireAnimationTrigger: row([lb("fire animation trigger"), slot("AnimTrigger", "name")]),
            SetRigSlot: row([lb("set rig slot"), slot("RigSlot", "slot"), lb("to attachment"), slot("RigAttachment", "attachment")]),
            SetSlotTint: row([lb("tint rig slot"), slot("RigSlot", "slot"), lb("with"), slot("SlotColor", "color")]),
            SetIkTarget: row([lb("point IK"), slot("IkConstraint", "constraint"), lb("at x"), slot("IkX", "x"), lb("y"), slot("IkY", "y")]),
            SetSpriteDial: row([lb("set sprite"), dd("dial", () => opts(["FlipX","FlipY","Order","YSort","Palette","OutlineWidth"])), lb("to"), slot("SpriteValue", "value")]),
            // Sound
            PlaySound: row([lb("play sound"), slot("SoundAsset", "sound"), lb("volume"), slot("SoundVolume", "volume"), lb("pitch"), slot("SoundPitch", "pitch"), flag("loop", soundLoopOptions), lb("on"), dd("bus", soundBusOptions)]),
            PlaySoundAt: row([lb("play sound"), slot("SoundAsset", "sound"), lb("volume"), slot("SoundVolume", "volume"), lb("pitch"), slot("SoundPitch", "pitch"), flag("loop", soundLoopOptions), lb("on"), dd("bus", soundBusOptions), lb("at"), slot("SoundTarget", "target")]),
            StopSound: row([lb("stop sound"), slot("SoundAsset", "sound")]),
            SetSoundVolume: row([lb("set volume of sound"), slot("SoundAsset", "sound"), lb("to"), slot("SoundVolume", "volume")]),
            SetSoundPitch: row([lb("set pitch of sound"), slot("SoundAsset", "sound"), lb("to"), slot("SoundPitch", "pitch")]),
            SetBusVolume: row([lb("set"), dd("bus", soundBusOptions), lb("volume to"), slot("SoundVolume", "volume")]),
            // Components
            SetComponentField: row([lb("set"), dd("field", componentFieldOptions, { placeholder: "field" }), lb("of"), dd("component", () => opts(customComponents().map(c => c.name)), { placeholder: "component" }), lb("to"), slot("ComponentFieldValue", "value")]),
            SetCameraView: row([lb("set my camera to"), dd("view", cameraViewOptions)]),
            SetCameraPitch: row([lb("set my camera pitch to"), slot("CameraPitchDegrees", "degrees")]),
            SetCameraFov: row([lb("set my camera fov to"), slot("CameraFovDegrees", "fov")]),
            AttachComponent: row([lb("attach"), dd("component", attachableOptions, { placeholder: "component" })]),
            DetachComponent: row([lb("detach"), dd("component", detachableOptions, { placeholder: "component" })]),
            SetParent: row([lb("attach me to"), slot("ParentTarget", "parent")]),
            // Actors
            CreateClone: row([lb("create a clone of"), dd("of", () => [{ value: "", label: "myself" }].concat(actorOptions(false)), { placeholder: "myself" })]),
            CreateActor: row([lb("create actor"), slot("NewActorName", "name"), lb("at x:"), slot("NewActorX", "x"), lb("y:"), slot("NewActorY", "y"), lb("z:", true), slot("NewActorZ", "z", { when: is3d })]),
            DeleteActor: row([lb("delete"), slot("DeleteTarget", "target")]),
            // Control
            Wait: row([lb("wait"), slot("WaitDuration", "duration"), lb("seconds")]),
            WaitUntil: row([lb("wait until"), slot("WaitUntilCondition", "condition", { bool: true })]),
            If: { head: [lb("if"), slot("Condition", "condition", { bool: true }), lb("then")], mouths: ["body"] },
            IfElse: { head: [lb("if"), slot("Condition", "condition", { bool: true }), lb("then")], mouths: ["then_body", "else_body"], separators: ["else"] },
            Repeat: { head: [lb("repeat"), slot("RepeatCount", "count")], mouths: ["body"] },
            Forever: { head: [lb("forever")], mouths: ["body"] },
            While: { head: [lb("while"), slot("Condition", "condition", { bool: true })], mouths: ["body"] },
            EscapeLoop: cap([lb("break out of the loop")]),
            ContinueLoop: cap([lb("next loop iteration")]),
            StopAll: cap([lb("stop everything")]),
            SetMouseLocked: row([flagDefaultTrue("locked", mouseLockOptions), lb("mouse")]),
            RumbleGamepad: row([lb("rumble gamepad at"), slot("RumbleStrength", "strength"), lb("for"), slot("RumbleDuration", "duration"), lb("secs")]),
            BindAction: row([lb("bind"), slot("ActionBinding", "binding"), lb("to action"), slot("ActionName", "action")]),
            ClearActionBindings: row([lb("clear bindings of action"), slot("ActionName", "action")]),
            // Interface
            ShowPanel: row([lb("show panel"), slot("UiId", "element"), slot("UiContent", "title"), flag("modal", modalOptions)].concat(placement())),
            ShowLabel: row(showRow("show label", "text")),
            ShowButton: row(showRow("show button", "label")),
            ShowImage: row(showRow("show image", "asset")),
            ShowInput: row(showRow("show text input", "placeholder")),
            ShowSlider: row(showRow("show slider", "", [lb("min"), slot("UiMin", "min"), lb("max"), slot("UiMax", "max"), lb("value"), slot("UiValue", "value")])),
            ShowToggle: row(showRow("show toggle", "label", [flag("on", onOffOptions)])),
            ShowWidget: row([lb("show"), dd("kind", () => opts(["VerticalBox", "HorizontalBox", "Grid", "Canvas", "WrapBox", "SizeBox", "Spacer", "Progress", "RadialProgress", "ListView", "Tabs", "Select", "Scrollbar", "RichText", "Tooltip"])), slot("UiId", "element"), slot("UiContent", "text")].concat(placement())),
            ShowList: row([lb("show list"), slot("UiId", "element")].concat(placement())),
            SetUiTheme: row([lb("set ui theme"), dd("theme", uiThemeOptions)]),
            BindUi: row([lb("bind"), slot("UiTarget", "element"), lb("to"), slot("UiPropValue", "value")]),
            SetUiItems: row([lb("set items of"), slot("UiTarget", "element"), lb("to"), slot("UiPropValue", "value")]),
            ScrollUi: row([lb("scroll"), slot("UiTarget", "element"), lb("to"), slot("UiPropValue", "value")]),
            SetElementTheme: row([lb("set theme of"), slot("UiTarget", "element"), lb("to"), slot("UiPropValue", "value")]),
            SetUiProp: row([lb("set"), dd("prop", uiPropOptions), lb("of"), slot("UiTarget", "element"), lb("to"), slot("UiPropValue", "value")]),
            HideElement: row([lb("hide"), slot("UiTarget", "element")]),
            HideAllUi: row([lb("hide all ui")]),
            DeleteElement: row([lb("delete"), slot("UiTarget", "element")]),
            FocusElement: row([lb("focus"), slot("UiTarget", "element")]),
            ClearFocus: row([lb("clear focus")]),
            PauseGame: row([lb("pause game")]),
            ResumeGame: row([lb("resume game")]),
            // Variables
            SetVariable: row([lb("set"), varD(), lb("to"), slot("SetVariableValue", "value")]),
            ChangeVariable: row([lb("change"), varD(), lb("by"), slot("ChangeVariableValue", "value")]),
            SaveVariable: row([lb("save"), varD()]),
            ClearSavedVariable: row([lb("clear saved"), varD()]),
            // Lists
            AddToList: row([lb("add"), slot("AddToListValue", "value"), lb("to"), listD()]),
            DeleteOfList: row([lb("delete item"), slot("DeleteOfListIndex", "index"), lb("of"), listD()]),
            DeleteAllOfList: row([lb("delete all of"), listD()]),
            ShiftList: row([lb("shift"), listD(), lb("by"), slot("ShiftListAmount", "amount")]),
            InsertIntoList: row([lb("insert"), slot("InsertIntoListValue", "value"), lb("at"), slot("InsertIntoListIndex", "index"), lb("of"), listD()]),
            ReplaceItemOfList: row([lb("replace item"), slot("ReplaceItemOfListIndex", "index"), lb("of"), listD(), lb("with"), slot("ReplaceItemOfListValue", "value")]),
            ReverseList: row([lb("reverse"), listD()]),
            LoadJsonIntoList: row([lb("load JSON"), slot("LoadJsonIntoListText", "json"), lb("into"), listD()]),
            // Dicts
            SetDictValue: row([lb("set"), slot("SetDictKey", "key"), lb("of"), dictD(), lb("to"), slot("SetDictValue", "value")]),
            DeleteDictKey: row([lb("delete"), slot("DeleteDictKey", "key"), lb("of"), dictD()]),
            DeleteAllOfDict: row([lb("delete all of"), dictD()]),
            LoadJsonIntoDict: row([lb("load JSON"), slot("LoadJsonIntoDictText", "json"), lb("into"), dictD()]),
            Return: cap([lb("return"), slot("ReturnValue", "value")]),
            // One generic type for every plugin block: its row is the schema's label.
            PluginBlock: { head: pluginHead },
            WhenPlugin: header(pluginHatHead)
        };
        for (const type in r) r[type].icon = icons[type] || "blocks";
        return r;
    }

    // ─── Plugin blocks ─────────────────────────────────────────────────────
    // What installed plugins add, from the snapshot's `plugins.blocks`:
    // [{ plugin, block: BlockSchema }]. A statement is the one generic
    // `PluginBlock` row, a hat the generic `WhenPlugin` header (its slots are
    // literal text a fired event is matched against, blank for any), and a
    // reporter the generic `PluginRead` operator whose first two args name
    // the plugin and block and the rest are its slots.
    readonly property var pluginBlocks: (appState && appState.plugins ? appState.plugins.blocks : null) || []
    readonly property string pluginSource: JSON.stringify(pluginBlocks.map(b => [b.plugin, b.block.type_id, b.block.kind, b.block.label, b.block.category, b.block.event || "", b.block.returns ? b.block.returns.type : "", b.block.slots.map(sl => [sl.name, sl.type, sl.default === undefined ? null : sl.default])]))
    onPluginSourceChanged: { pluginPieces = ({}); BlockRegistry.revision++; }
    // The row's pieces per block. The canvas rebuilds a row's controls when
    // the array changes, so the same objects come back until the plugins do.
    property var pluginPieces: ({})
    function pluginSchema(plugin, block) {
        for (const b of pluginBlocks) if (b.plugin === plugin && b.block.type_id === block) return b.block;
        return null;
    }
    // A label's words and slots in order: "set {actor} health to {value}".
    // `slotPiece(index, slot)` makes the piece for a slot.
    function pluginLabelPieces(schema, slotPiece) {
        const pieces = [];
        schema.label.split(/\{([^}]*)\}/).forEach((part, i) => {
            if (i % 2 === 0) {
                if (part.trim().length) pieces.push(lb(part.trim()));
                return;
            }
            const index = schema.slots.findIndex(sl => sl.name === part);
            if (index >= 0) pieces.push(slotPiece(index, schema.slots[index]));
        });
        return pieces;
    }
    function pluginHead(ins) {
        const key = ins.plugin + "/" + ins.block;
        if (pluginPieces[key]) return pluginPieces[key];
        const schema = pluginSchema(ins.plugin, ins.block);
        const pieces = schema
            ? pluginLabelPieces(schema, (index) => ({ kind: "value", field: "PluginArg:" + index, key: "args", index: index }))
            : [lb(key + " (not installed)")];
        pluginPieces[key] = pieces;
        return pieces;
    }
    function pluginHatHead(ins) {
        const key = "hat:" + ins.plugin + "/" + ins.block;
        if (pluginPieces[key]) return pluginPieces[key];
        const schema = pluginSchema(ins.plugin, ins.block);
        const pieces = schema
            ? pluginLabelPieces(schema, (index, slot) => ({ kind: "text", key: "args", index: index, placeholder: slot.name }))
            : [lb(ins.plugin + "/" + ins.block + " (not installed)")];
        pluginPieces[key] = pieces;
        return pieces;
    }
    // A reporter's words and slots; its args start after the plugin and block.
    function pluginLayout(value) {
        const args = value && value.args ? value.args : [];
        const plugin = args[0] ? args[0].value : "", block = args[1] ? args[1].value : "";
        const key = "read:" + plugin + "/" + block;
        if (pluginPieces[key]) return pluginPieces[key];
        const schema = pluginSchema(plugin, block);
        const pieces = schema
            ? pluginLabelPieces(schema, (index, slot) => ({ label: "", arg: index + 2, bool: slot.type === "bool" })).map(p => p.kind === "label" ? { label: p.text, arg: -1, bool: false } : p)
            : [{ label: plugin + "/" + block + " (not installed)", arg: -1, bool: false }];
        pluginPieces[key] = pieces;
        return pieces;
    }
    function pluginResult(value) {
        const args = value && value.args ? value.args : [];
        const schema = pluginSchema(args[0] ? args[0].value : "", args[1] ? args[1].value : "");
        const type = schema && schema.returns ? schema.returns.type : "";
        return type === "bool" ? "bool" : (type === "int" || type === "number") ? "number" : "text";
    }
    function pluginDefault(slot) {
        const d = slot.default;
        switch (slot.type) {
        case "bool": return { kind: "Bool" };
        case "int": case "number": return num(d !== undefined && d !== null ? d : (slot.min !== undefined ? slot.min : 0));
        case "choice": return txt(d !== undefined && d !== null ? d : (slot.options.length ? slot.options[0] : ""));
        default: return txt(d !== undefined && d !== null && typeof d === "string" ? d : "");
        }
    }
    // A statement or a hat as the palette and canvas hold it.
    function pluginFresh(entry) {
        const schema = entry.block;
        if (schema.kind === "hat")
            return { id: uuid(), type: "WhenPlugin", plugin: entry.plugin, block: schema.type_id, event: schema.event || "", args: schema.slots.map(sl => sl.default === undefined || sl.default === null ? "" : String(sl.default)) };
        return { id: uuid(), type: "PluginBlock", plugin: entry.plugin, block: schema.type_id, args: schema.slots.map(pluginDefault) };
    }
    // A reporter as a value.
    function pluginValue(entry) {
        return { kind: "Op", op: "PluginRead", args: [txt(entry.plugin), txt(entry.block.type_id)].concat(entry.block.slots.map(pluginDefault)), saved: num(0) };
    }
    function pluginIsBool(entry) { return !!entry.block.returns && entry.block.returns.type === "bool"; }
    // The palette's sections, one per category the plugins name: the blocks
    // (statements and hats) and the reporters of each.
    readonly property var pluginGroups: {
        const groups = [];
        for (const entry of pluginBlocks) {
            let g = groups.find(x => x.label === entry.block.category);
            if (!g) { g = { label: entry.block.category, entries: [], reporters: [] }; groups.push(g); }
            (entry.block.kind === "reporter" ? g.reporters : g.entries).push(entry);
        }
        return groups;
    }

    // ─── Fresh palette blocks (paletteState.ts) ────────────────────────────
    function num(v) { return { kind: "Number", value: v }; }
    function txt(v) { return { kind: "Text", value: v }; }
    function uiPlacement(anchor) { return { anchor: anchor, x: num(0), y: num(0), width: num(0), height: num(0), parent: txt("") }; }
    function defaults(type) {
        const blank = { kind: "Bool" };
        switch (type) {
        case "WhenKeyPressed": return { key: "space" };
        case "WhenActionPressed": return { action: "Jump" };
        case "WhenCollision": return { with: "", phase: "Enter", scope: "Any" };
        case "WhenMessage": case "Broadcast": return { name: "message1" };
        case "BlockHeader": return { block_id: "" };
        case "Move": return { steps: num(10) };
        case "GoTo": return { x: num(0), y: num(0), z: num(0) };
        case "NavigateTo": return { x: num(0), y: num(0), z: num(0), speed: num(4) };
        case "ChangePosition": return { axis: "X", by: num(10) };
        case "Glide": return { seconds: num(1), x: num(0), y: num(0), z: num(0), easing: "Linear" };
        case "TweenScale": return { factor: num(1), seconds: num(1), easing: "Linear" };
        case "TweenRotation": return { axis: "Z", degrees: num(90), seconds: num(1), easing: "EaseOut" };
        case "TweenColor": return { color: txt("#FFAB19"), seconds: num(1), easing: "EaseOut" };
        case "PlayAnimation": return { clip: txt("walk"), speed: num(1) };
        case "SetAnimationSpeed": return { speed: num(1) };
        case "WhenAnimationEnds": return { clip: "" };
        case "WhenAnimationMarker": return { marker: "" };
        case "WhenEnterRoom": return { room: "" };
        case "FireAnimationTrigger": return { name: txt("jump") };
        case "SetRigSlot": return { slot: txt("hand"), attachment: txt("fist") };
        case "SetSlotTint": return { slot: txt("cape"), color: txt("#FF4C4C") };
        case "SetIkTarget": return { constraint: txt("reach"), x: num(40), y: num(0) };
        case "SetSpriteDial": return { dial: "FlipX", value: num(1) };
        case "Turn": return { axis: "Z", degrees: num(15) };
        case "SetRotation": return { axis: "Z", degrees: num(0) };
        case "PointTowards": return { target: "mouse" };
        case "SetScale": return { factor: num(1) };
        case "SetBody": return { body: "Dynamic" };
        case "ApplyImpulse": return { x: num(0), y: num(5), z: num(0) };
        case "AddForce": return { mode: "Force", x: num(0), y: num(10), z: num(0) };
        case "MotorAct": return { action: "Intent", x: num(0), y: num(0), z: num(0) };
        case "JointAct": return { action: "MotorSpeed", joint: "1", value: num(90) };
        case "SetMotor": return { property: "WalkSpeed", value: num(5) };
        case "ControllerMove": return { mode: "Simple", x: num(0), y: num(0), z: num(0) };
        case "SetController": return { property: "Radius", value: num(0.5) };
        case "AddTorque": return { mode: "Force", x: num(0), y: num(0), z: num(1) };
        case "CastRay": return { hits: "Nearest", triggers: "UseGlobal", from_x: num(0), from_y: num(0), from_z: num(0), to_x: num(0), to_y: num(-10), to_z: num(0) };
        case "CastBall": return { triggers: "UseGlobal", radius: num(0.5), from_x: num(0), from_y: num(0), from_z: num(0), to_x: num(0), to_y: num(-10), to_z: num(0) };
        case "OverlapBall": return { triggers: "UseGlobal", radius: num(1), x: num(0), y: num(0), z: num(0) };
        case "FindClosest": return { triggers: "UseGlobal", range: num(10), x: num(0), y: num(0), z: num(0) };
        case "SetVelocity": return { x: num(0), y: num(0), z: num(0) };
        case "SetGravity": return { x: num(0), y: num(-9.81), z: num(0) };
        case "SetDensity": return { density: num(1) };
        case "SetMass": return { mass: num(1) };
        case "SetTrigger": return { trigger: false };
        case "SetCollisionLayer": return { layer: num(1) };
        case "SetCollisionMask": return { mask: num(255) };
        case "Say": return { text: txt("Hello!") };
        case "SetVisible": return { visible: true };
        case "SetColor": return { color: txt("#FFAB19") };
        case "SetRenderSetting": return { setting: "Quality", value: txt("High") };
        case "SetExposure": return { ev: num(9.7) };
        case "SetLightIntensity": return { intensity: num(800) };
        case "SetEmissiveStrength": return { strength: num(2) };
        case "SetHdrOutput": return { enabled: true };
        case "SetPeakBrightness": return { nits: num(1000) };
        case "EnableVolume": return { enabled: true, volume: txt("") };
        case "SetVolumeWeight": return { volume: txt(""), weight: num(1) };
        case "CaptureProbes": return {};
        case "SetShadowDistance": return { distance: num(50) };
        case "SetLightShadows": return { enabled: true };
        case "SetRayTracing": return { enabled: true };
        case "SetGiBounces": return { bounces: num(3) };
        case "SetGiSamples": return { samples: num(8) };
        case "SetFogDensity": return { density: num(0.01) };
        case "SetAurora": return { kp: num(5) };
        case "StrikeLightning": return { x: num(0), y: num(0), z: num(0) };
        case "Fracture": return { target: txt("myself") };
        case "Splash": return { x:num(0),y:num(0),z:num(0),radius:num(1),strength:num(0.2) };
        case "PuffSmoke": return { x:num(0),y:num(0),z:num(0),radius:num(2),strength:num(1) };
        case "SpawnDecal": return { preset: "Blood", x: num(0), y: num(0), z: num(0), nx: num(0), ny: num(1), nz: num(0), size: num(1), lifetime: num(20), fade: num(3) };
        case "FadeDecals": return { x: num(0), y: num(0), z: num(0), radius: num(5), seconds: num(1) };
        case "SetLightningRate": return { rate: num(6) };
        case "SetWind": return { property: "Speed", value: num(5) };
        case "SetClouds": return { property: "Coverage", value: num(0.5) };
        case "WhenWeather": return { weather: "" };
        case "SetTimeOfDay": return { time: num(12) };
        case "AdvanceTime": return { hours: num(1) };
        case "SetPrecipitation": return { property: "Rain", value: num(0.5) };
        case "BlendWeather": return { weather: txt("Storm"), seconds: num(5) };
        case "SetWater": return { property: "Level", value: num(0) };
        case "PaintTile": return { map: txt(""), tile: num(0), x: num(0), y: num(0), z: num(0) };
        case "SetParallax": return { layer: txt("Background"), axis: "Both", value: num(0.5) };
        case "SetCloudLayer": return { layer: num(1), property: "Coverage", value: num(0.5) };
        case "SetCloudDrift": return { x: num(0), y: num(0), z: num(0) };
        case "BurstParticles": return { count: num(24) };
        case "SetEmitterDial": return { dial: "Rate", value: num(24) };
        case "SetTrailEnabled": return { enabled: true };
        case "SetEmitterPlaying": return { playing: true };
        case "WhenParticles": return { event: "Spawn" };
        case "SwitchScene": return { scene: txt("Scene 2"), transition: txt("none") };
        case "PlayCutscene": return { cutscene: txt("Opener") };
        case "CameraShake": return { amount: num(0.5) };
        case "SetTimeScale": return { scale: num(0.5) };
        case "Hitstop": return { frames: num(6) };
        case "SetLetterbox": return { on: num(1) };
        case "FadeScreen": return { color: txt("black") };
        case "WhenCutsceneSignal": return { signal: "" };
        case "PlaySound": return { sound: txt("assets/sounds/sound.wav"), volume: num(100), pitch: num(1), loop: false, bus: "Sfx" };
        case "PlaySoundAt": return { sound: txt("assets/sounds/sound.wav"), volume: num(100), pitch: num(1), loop: false, bus: "Sfx", target: txt("") };
        case "StopSound": return { sound: txt("") };
        case "SetSoundVolume": return { sound: txt(""), volume: num(100) };
        case "SetSoundPitch": return { sound: txt(""), pitch: num(1) };
        case "SetBusVolume": return { bus: "Sfx", volume: num(100) };
        case "SetComponentField": return { component: "", field: "", value: num(0) };
        case "SetCameraView": return { view: "ThirdPerson" };
        case "SetCameraPitch": return { degrees: num(0) };
        case "SetCameraFov": return { fov: num(75) };
        case "AttachComponent": case "DetachComponent": return { component: "" };
        case "SetParent": return { parent: txt("") };
        case "CreateClone": return { of: "" };
        case "CreateActor": return { name: txt("Actor"), x: num(0), y: num(0), z: num(0) };
        case "DeleteActor": return { target: txt("myself") };
        case "Wait": return { duration: num(1) };
        case "WaitUntil": return { condition: blank };
        case "If": return { condition: blank, body: [] };
        case "IfElse": return { condition: blank, then_body: [], else_body: [] };
        case "Repeat": return { count: num(10), body: [] };
        case "Forever": return { body: [] };
        case "While": return { condition: blank, body: [] };
        case "SetVariable": return { name: variableNames()[0] || "", value: num(0) };
        case "ChangeVariable": return { name: variableNames()[0] || "", value: num(1) };
        case "CallBlock": return { block_id: "", args: [] };
        case "PluginBlock": return { plugin: "", block: "", args: [] };
        case "WhenPlugin": return { plugin: "", block: "", event: "", args: [] };
        case "Return": return { value: num(0) };
        case "SetMouseLocked": return { locked: true };
        case "RumbleGamepad": return { strength: num(100), duration: num(0.5) };
        case "BindAction": return { action: txt("Jump"), binding: txt("space") };
        case "ClearActionBindings": return { action: txt("Jump") };
        case "WhenUiEvent": return { element: "my-button", event: "hover" };
        case "WhenUiClicked": case "WhenUiChanged": return { element: "my-button" };
        case "ShowPanel": return Object.assign(uiPlacement("Center"), { element: txt("menu"), title: txt("Menu"), modal: true });
        case "ShowLabel": return Object.assign(uiPlacement("TopLeft"), { element: txt("score"), text: txt("Score: 0") });
        case "ShowButton": return Object.assign(uiPlacement("Center"), { element: txt("resume"), label: txt("Resume") });
        case "ShowImage": return Object.assign(uiPlacement("TopLeft"), { element: txt("logo"), asset: txt("") });
        case "ShowInput": return Object.assign(uiPlacement("Center"), { element: txt("name"), placeholder: txt("your name") });
        case "ShowSlider": return Object.assign(uiPlacement("Center"), { element: txt("volume"), min: num(0), max: num(100), value: num(50) });
        case "ShowToggle": return Object.assign(uiPlacement("Center"), { element: txt("shadows"), label: txt("Shadows"), on: true });
        case "ShowWidget": return Object.assign(uiPlacement("TopLeft"), { kind: "VerticalBox", element: txt("widget"), text: txt("") });
        case "ShowList": return Object.assign(uiPlacement("Center"), { element: txt("items"), width: num(280), height: num(240) });
        case "SetUiTheme": return { theme: "Dark" };
        case "BindUi": return { element: txt("widget"), value: txt("[]") };
        case "SetUiItems": return { element: txt("widget"), value: txt("[]") };
        case "ScrollUi": return { element: txt("widget"), value: txt("0") };
        case "SetElementTheme": return { element: txt("widget"), value: txt("Dark") };
        case "SetUiProp": return { prop: "Text", element: txt("score"), value: txt("") };
        case "SaveVariable": case "ClearSavedVariable": return { name: variableNames()[0] || "" };
        case "AddToList": return { name: listNames()[0] || "", value: num(0) };
        case "DeleteOfList": return { name: listNames()[0] || "", index: num(1) };
        case "DeleteAllOfList": case "ReverseList": return { name: listNames()[0] || "" };
        case "ShiftList": return { name: listNames()[0] || "", amount: num(1) };
        case "InsertIntoList": return { name: listNames()[0] || "", value: num(0), index: num(1) };
        case "ReplaceItemOfList": return { name: listNames()[0] || "", index: num(1), value: num(0) };
        case "LoadJsonIntoList": return { name: listNames()[0] || "", json: txt("") };
        case "SetDictValue": return { name: dictNames()[0] || "", key: txt(""), value: num(0) };
        case "DeleteDictKey": return { name: dictNames()[0] || "", key: txt("") };
        case "DeleteAllOfDict": return { name: dictNames()[0] || "" };
        case "LoadJsonIntoDict": return { name: dictNames()[0] || "", json: txt("") };
        case "HideElement": case "DeleteElement": return { element: txt("menu") };
        case "FocusElement": return { element: txt("name") };
        default: return {};
        }
    }
    function uuid() { return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, c => { const r = Math.random() * 16 | 0; return (c === "x" ? r : (r & 3 | 8)).toString(16); }); }
    // A fresh block of `type`, with the defaults that make it useful straight away.
    function fresh(type) { return Object.assign({ id: uuid(), type: type }, defaults(type)); }
    // A copy with new ids all the way down, for duplicating and pasting.
    function regenerateIds(ins) {
        const copy = JSON.parse(JSON.stringify(ins));
        const walk = i => {
            i.id = uuid();
            for (const key of ["body", "then_body", "else_body"]) if (Array.isArray(i[key])) i[key].forEach(walk);
        };
        walk(copy);
        return copy;
    }

    // ─── Sensing reporters, registered in blockloom-core/src/value.rs ───────
    readonly property var sensing: ({
        KeyDown: { prefix: "key", suffix: "down?", result: "bool", enumArg: { index: 0, options: keyOptions }, arity: 1, args: ["text"] },
        MouseDown: { prefix: "mouse down?", result: "bool", arity: 0 },
        MouseButtonDown: { prefix: "mouse button", suffix: "down?", result: "bool", enumArg: { index: 0, options: mouseButtonOptions }, arity: 1, args: ["text"] },
        MouseX: { prefix: "mouse x", result: "number", arity: 0 },
        MouseY: { prefix: "mouse y", result: "number", arity: 0 },
        MouseDeltaX: { prefix: "mouse delta x", result: "number", arity: 0 },
        MouseDeltaY: { prefix: "mouse delta y", result: "number", arity: 0 },
        MouseLocked: { prefix: "mouse locked?", result: "bool", arity: 0 },
        Timer: { prefix: "timer", result: "number", arity: 0 },
        UiValue: { prefix: "value of", result: "number", arity: 1, args: ["text"] },
        UiText: { prefix: "text of", result: "text", arity: 1, args: ["text"] },
        UiSelectedIndex: { prefix: "selected index of", result: "number", arity: 1, args: ["text"] },
        UiShown: { prefix: "is", suffix: "shown?", result: "bool", arity: 1, args: ["text"] },
        UiExists: { prefix: "does", suffix: "exist?", result: "bool", arity: 1, args: ["text"] },
        UiFocus: { prefix: "focused element", result: "text", arity: 0 },
        GamePaused: { prefix: "game paused?", result: "bool", arity: 0 },
        MyPosition: { prefix: "my", suffix: "position", result: "number", enumArg: { index: 0, options: axisOptions }, arity: 1, args: ["text"] },
        CameraPosition: { prefix: "camera", suffix: "position", result: "number", enumArg: { index: 0, options: axisOptions }, arity: 1, args: ["text"] },
        CameraDirection: { prefix: "camera", suffix: "direction", result: "number", enumArg: { index: 0, options: axisOptions }, arity: 1, args: ["text"] },
        MyRotation: { prefix: "my", suffix: "rotation", result: "number", enumArg: { index: 0, options: axisOptions }, arity: 1, args: ["text"] },
        MyLocalPosition: { prefix: "my local", suffix: "position", result: "number", enumArg: { index: 0, options: axisOptions }, arity: 1, args: ["text"] },
        Touching: { prefix: "touching", result: "bool", arity: 1, args: ["text"] },
        ActionDown: { prefix: "action", suffix: "down?", result: "bool", arity: 1, args: ["text"] },
        ActionPressed: { prefix: "action", suffix: "pressed?", result: "bool", arity: 1, args: ["text"] },
        ActionReleased: { prefix: "action", suffix: "released?", result: "bool", arity: 1, args: ["text"] },
        ActionValue: { prefix: "action value", result: "number", arity: 1, args: ["text"] },
        TouchCount: { prefix: "touch count", result: "number", arity: 0 },
        TouchX: { prefix: "touch", suffix: "x", result: "number", arity: 1, args: ["number"] },
        TouchY: { prefix: "touch", suffix: "y", result: "number", arity: 1, args: ["number"] },
        GamepadConnected: { prefix: "gamepad connected?", result: "bool", arity: 0 },
        GamepadAxis: { prefix: "gamepad axis", result: "number", enumArg: { index: 0, options: gamepadAxisOptions }, arity: 1, args: ["text"] },
        GamepadButtonDown: { prefix: "gamepad button", suffix: "down?", result: "bool", enumArg: { index: 0, options: gamepadButtonOptions }, arity: 1, args: ["text"] },
        IsTrigger: { prefix: "is", suffix: "a trigger?", result: "bool", arity: 1, args: ["text"] },
        CollisionLayer: { prefix: "collision layer of", result: "number", arity: 1, args: ["text"] },
        RayHit: { prefix: "ray from", suffix: "hits", result: "text", arity: 6, args: ["number","number","number","number","number","number"] },
        QueryNumber: { prefix: "hit", infix: "'s", result: "number", enumArg: { index: 1, options: hitNumberOptions }, arity: 2, args: ["number","text"] },
        ControllerNumber: { prefix: "controller", infix: "'s", result: "number", enumArg: { index: 1, options: controllerNumberOptions }, arity: 2, args: ["number","text"] },
        MotorNumber: { prefix: "motor", result: "number", enumArg: { index: 0, options: motorNumberOptions }, arity: 1, args: ["text"] },
        JointNumber: { prefix: "joint", result: "number", enumArg: { index: 0, options: jointNumberOptions }, arity: 2, args: ["text", "text"] },
        MotorText: { prefix: "motor", result: "text", enumArg: { index: 0, options: motorTextOptions }, arity: 1, args: ["text"] },
        ControllerText: { prefix: "controller", infix: "'s", result: "text", enumArg: { index: 1, options: controllerTextOptions }, arity: 2, args: ["number","text"] },
        QueryText: { prefix: "hit", infix: "'s", result: "text", enumArg: { index: 1, options: hitTextOptions }, arity: 2, args: ["number","text"] },
        RayDistance: { prefix: "ray distance from", result: "number", arity: 6, args: ["number","number","number","number","number","number"] },
        CircleHit: { prefix: "ball at", suffix: "overlaps", result: "text", arity: 4, args: ["number","number","number","number"] },
        DistanceTo: { prefix: "distance to", result: "number", arity: 1, args: ["text"] },
        ComponentField: { prefix: "my", infix: "field", result: "number", arity: 2, args: ["text","text"] },
        IsClone: { prefix: "am I a clone?", result: "bool", arity: 0 },
        MyParent: { prefix: "the actor I hang off", result: "text", arity: 0 },
        NewActor: { prefix: "the actor I made", result: "text", arity: 0 },
        ActorCount: { prefix: "how many", suffix: "there are", result: "number", arity: 1, args: ["text"] },
        ActorPosition: { infix: "'s", suffix: "position", result: "number", enumArg: { index: 1, options: axisOptions }, arity: 2, args: ["text","text"] },
        ActorLocalPosition: { infix: "'s local", suffix: "position", result: "number", enumArg: { index: 1, options: axisOptions }, arity: 2, args: ["text","text"] },
        Velocity: { infix: "'s", suffix: "velocity", result: "number", enumArg: { index: 1, options: axisOptions }, arity: 2, args: ["text","text"] },
        AngularVelocity: { infix: "'s", suffix: "angular velocity", result: "number", enumArg: { index: 1, options: axisOptions }, arity: 2, args: ["text","text"] },
        Mass: { prefix: "mass of", result: "number", arity: 1, args: ["text"] },
        IsGrounded: { prefix: "is", suffix: "grounded?", result: "bool", arity: 1, args: ["text"] },
        SoundPlaying: { prefix: "is", suffix: "playing?", result: "bool", arity: 1, args: ["text"] },
        BusVolume: { prefix: "volume of", suffix: "bus", result: "number", enumArg: { index: 0, options: soundBusOptions }, arity: 1, args: ["text"] },
        IsTweening: { prefix: "tweening?", result: "bool", arity: 0 },
        CurrentClip: { prefix: "current clip", result: "text", arity: 0 },
        CurrentFrame: { prefix: "current frame", result: "number", arity: 0 },
        AnimationPlaying: { prefix: "animation playing?", result: "bool", arity: 0 },
        Atmosphere: { prefix: "air", result: "number", enumArg: { index: 0, options: atmosphereOptions }, arity: 1, args: ["text"] },
        FrameTime: { prefix: "frame time", result: "number", arity: 0 },
        DrawCalls: { prefix: "estimated mesh draws", result: "number", arity: 0 },
        CurrentQuality: { prefix: "current quality", result: "text", arity: 0 },
        DlssAvailable: { prefix: "DLSS available?", result: "bool", arity: 0 },
        SceneLuminance: { prefix: "scene luminance", result: "number", arity: 0 },
        IsHdrDisplay: { prefix: "HDR display?", result: "bool", arity: 0 },
        PeakBrightness: { prefix: "peak brightness", result: "number", arity: 0 },
        ActiveVolumes: { prefix: "active volumes", result: "text", arity: 0 },
        CurrentScene: { prefix: "current scene", result: "text", arity: 0 },
        SceneNames: { prefix: "scene names", result: "text", arity: 0 },
        IsCutscenePlaying: { prefix: "is cutscene playing?", result: "bool", arity: 0 },
        CutsceneTime: { prefix: "cutscene time", result: "number", arity: 0 },
        TimeOfDay: { prefix: "time of day", result: "number", arity: 0 },
        SunElevation: { prefix: "sun elevation", result: "number", arity: 0 },
        CurrentWeather: { prefix: "current weather", result: "text", arity: 0 },
        CastsShadows: { prefix: "does", suffix: "cast shadows?", result: "bool", arity: 1, args: ["text"] },
        IsRayTracing: { prefix: "ray tracing on?", result: "bool", arity: 0 },
        RayTracingAvailable: { prefix: "ray tracing available?", result: "bool", arity: 0 },
        WaterHeight: { prefix: "water height at", result: "number", arity: 2, args: ["number","number"] },
        Underwater: { prefix: "is", suffix: "underwater?", result: "bool", arity: 1, args: ["text"] },
        TileAt: { prefix: "tile at", suffix: "of map", result: "number", arity: 4, args: ["number","number","number","text"] },
        RoomContaining: { prefix: "room containing", result: "text", arity: 1, args: ["text"] },
        ParticleCount: { prefix: "my particle count", result: "number", arity: 0 },
        ParticleEventCount: { prefix: "how many of my particles", result: "number", enumArg: { index: 0, options: particleEventOptions }, arity: 1, args: ["text"] },
        // The axis is typed (x, y or z); only one slot of a reporter can be a dropdown.
        ParticleEventPosition: { prefix: "where my particles last", infix: "on axis", result: "number", enumArg: { index: 0, options: particleEventOptions }, arity: 2, args: ["text","text"] }
    })
    // Arity and slot types of blockstitch's own operators, for fresh palette values.
    readonly property var builtinShapes: ({
        Add:[2,"n"], Sub:[2,"n"], Mul:[2,"n"], Div:[2,"n"], Mod:[2,"n"], Round:[1,"n"], Math:[2,"tn"], Random:[2,"n"],
        Join:[2,"t"], Join3:[3,"t"], NewLine:[0,""], Tab:[0,""], IndexOf:[2,"t"], LastIndexOf:[2,"t"], LetterOf:[2,"nt"], Length:[1,"t"], Case:[2,"t"],
        Eq:[2,"n"], Neq:[2,"n"], Gt:[2,"n"], Lt:[2,"n"], Gte:[2,"n"], Lte:[2,"n"], And:[2,"b"], Or:[2,"b"], Not:[1,"b"], True:[0,""], False:[0,""],
        CurrentTime:[1,"t"],
        ListItem:[2,"nt"], ListItemNumber:[2,"t"], ListAmount:[2,"t"], ListLength:[1,"t"], ListContains:[2,"t"], ListItemExists:[2,"nt"], ListIsEmpty:[1,"t"], ListAsJson:[1,"t"],
        DictValue:[2,"t"], DictHasKey:[2,"t"], DictSize:[1,"t"], DictKeys:[1,"t"], DictAsJson:[1,"t"], DictIsEmpty:[1,"t"]
    })
    // Palette groups, the way the sidebar shows them.
    readonly property var operatorGroups: [
        { label: "Sensing", kinds: ["KeyDown","MouseDown","MouseButtonDown","MouseX","MouseY","MouseDeltaX","MouseDeltaY","MouseLocked","ActionDown","ActionPressed","ActionReleased","ActionValue","TouchCount","TouchX","TouchY","GamepadConnected","GamepadAxis","GamepadButtonDown","Timer","MyPosition","MyRotation","MyLocalPosition","CameraPosition","CameraDirection","Touching","DistanceTo","IsTrigger","CollisionLayer","RayHit","RayDistance","CircleHit","QueryNumber","QueryText","ControllerNumber","ControllerText","MotorNumber","MotorText","JointNumber","ActorPosition","ActorLocalPosition","Velocity","AngularVelocity","Mass","IsGrounded","ComponentField","SoundPlaying","BusVolume","IsTweening","CurrentClip","CurrentFrame","AnimationPlaying","Atmosphere","TimeOfDay","SunElevation","CurrentWeather","FrameTime","DrawCalls","CurrentQuality","DlssAvailable","SceneLuminance","IsHdrDisplay","PeakBrightness","ActiveVolumes","CurrentScene","SceneNames","IsCutscenePlaying","CutsceneTime","CastsShadows","IsRayTracing","RayTracingAvailable","WaterHeight","Underwater","TileAt","RoomContaining","ParticleCount","ParticleEventCount","ParticleEventPosition"] },
        { label: "Interface", kinds: ["UiSelectedIndex","UiValue","UiText","UiShown","UiExists","UiFocus","GamePaused"] },
        { label: "Actors", kinds: ["IsClone","MyParent","NewActor","ActorCount"] },
        { label: "Maths", kinds: ["Add","Sub","Mul","Div","Mod","Round","Math","Random"] },
        { label: "Comparing", kinds: ["Eq","Neq","Gt","Lt","Gte","Lte","And","Or","Not","True","False"] },
        { label: "Text", kinds: ["Join","Join3","Length","LetterOf","IndexOf","LastIndexOf","Case","NewLine","Tab","CurrentTime"] }
    ]
    readonly property var listOperatorKinds: ["ListItem","ListItemNumber","ListAmount","ListLength","ListContains","ListItemExists","ListIsEmpty","ListAsJson"]
    readonly property var dictOperatorKinds: ["DictValue","DictHasKey","DictSize","DictKeys","DictAsJson","DictIsEmpty"]
    readonly property var blockGroups: [
        { label: "Events", types: ["WhenStarted","WhenQualityDrops","WhenSceneStarts","WhenSceneEnds","WhenWeather","WhenCutsceneSignal","WhenCutsceneEnds","WhenKeyPressed","WhenActionPressed","WhenTouched","WhenClicked","WhenCollision","WhenMessage","WhenCloned","WhenParticles","WhenAnimationEnds","WhenAnimationMarker","WhenEnterRoom","WhenUiEvent","WhenUiClicked","WhenUiChanged","Broadcast"] },
        { label: "Motion", types: ["Move","GoTo","NavigateTo","ChangePosition","Glide","TweenScale","TweenRotation","TweenColor","StopTweens","Turn","SetRotation","PointTowards","SetScale"] },
        { label: "Physics", types: ["SetBody","ApplyImpulse","AddForce","AddTorque","ControllerMove","SetController","MotorAct","SetMotor","JointAct","CastRay","CastBall","OverlapBall","FindClosest","SetVelocity","SetGravity","SetDensity","SetMass","SetTrigger","SetCollisionLayer","SetCollisionMask"] },
        { label: "Looks", types: ["Say","SetVisible","SetColor","SetRenderSetting","SetExposure","SetLightIntensity","SetEmissiveStrength","SetHdrOutput","SetPeakBrightness","EnableVolume","SetVolumeWeight","CaptureProbes","SetShadowDistance","SetLightShadows","SetRayTracing","SetGiBounces","SetGiSamples","SetFogDensity","SetAurora","StrikeLightning","SetLightningRate","SetWind","SetCloudDrift","SetClouds","SetCloudLayer","SetWater","SetTimeOfDay","AdvanceTime","SetPrecipitation","BlendWeather","Fracture","Splash","PuffSmoke","SpawnDecal","FadeDecals","PaintTile","SetParallax","BurstParticles","SetEmitterDial","SetTrailEnabled","SetEmitterPlaying","PlayAnimation","StopAnimation","SetAnimationSpeed","FireAnimationTrigger","SetRigSlot","SetSlotTint","SetIkTarget","SetSpriteDial"] },
        { label: "Sound", types: ["PlaySound","PlaySoundAt","StopSound","SetSoundVolume","SetSoundPitch","SetBusVolume"] },
        { label: "Components", types: ["SetComponentField","SetCameraView","SetCameraPitch","SetCameraFov","AttachComponent","DetachComponent","SetParent"] },
        { label: "Actors", types: ["CreateClone","CreateActor","DeleteActor"] },
        { label: "Interface", types: ["ShowWidget","ShowPanel","ShowLabel","ShowButton","ShowImage","ShowInput","ShowSlider","ShowToggle","ShowList","SetUiTheme","BindUi","SetUiItems","ScrollUi","SetElementTheme","SetUiProp","HideElement","HideAllUi","DeleteElement","FocusElement","ClearFocus","PauseGame","ResumeGame"] },
        { label: "Control", types: ["Wait","WaitUntil","If","IfElse","Repeat","Forever","While","EscapeLoop","ContinueLoop","StopAll","SwitchScene","PlayCutscene","SkipCutscene","CameraShake","SetTimeScale","Hitstop","SetLetterbox","FadeScreen","SetMouseLocked"] },
        { label: "Input", types: ["RumbleGamepad","BindAction","ClearActionBindings"] }
    ]
    readonly property var variableCommandTypes: ["SetVariable","ChangeVariable","SaveVariable","ClearSavedVariable"]
    readonly property var listCommandTypes: ["AddToList","DeleteOfList","DeleteAllOfList","ShiftList","InsertIntoList","ReplaceItemOfList","ReverseList","LoadJsonIntoList"]
    readonly property var dictCommandTypes: ["SetDictValue","DeleteDictKey","DeleteAllOfDict","LoadJsonIntoDict"]

    function isBoolKind(kind) {
        const s = sensing[kind];
        if (s) return s.result === "bool";
        return BlockRegistry.isBoolOp(kind === "Join3" ? "Join" : kind);
    }
    // A fresh operator value: dropdown slots start at their first choice,
    // list indexes at one, booleans blank, text empty, numbers zero.
    function operatorValue(kind) {
        const op = kind === "Join3" ? "Join" : kind;
        const s = sensing[kind];
        const spec = BlockRegistry.operator(op);
        let arity = 0, types = [];
        if (s) { arity = s.arity; types = s.args || []; }
        else if (builtinShapes[kind]) {
            const shape = builtinShapes[kind];
            arity = shape[0];
            for (let i = 0; i < arity; ++i) types.push(shape[1].length > 1 ? shape[1][i] : shape[1]);
        }
        const args = [];
        for (let i = 0; i < arity; ++i) {
            const t = types[i];
            if (spec && spec.enumArg && spec.enumArg.index === i) args.push(txt((BlockRegistry.enumChoices(spec.enumArg)[0] || { value: "" }).value));
            else if (spec && spec.oneBased && spec.oneBased.indexOf(i) >= 0) args.push(num(1));
            else if (t === "b" || t === "bool") args.push({ kind: "Bool" });
            else if (t === "t" || t === "text") args.push(txt(""));
            else args.push(num(0));
        }
        return { kind: "Op", op: op, args: args, saved: num(0) };
    }

    // What the Details dialog says about a block.
    readonly property var labels: ({
        Fracture:"fracture an actor into debris", Splash:"make a water ripple", PuffSmoke:"puff advected smoke", SpawnDecal:"spawn a transient surface decal", FadeDecals:"fade decals in a radius",
        WhenStarted:"when the project starts", WhenKeyPressed:"when a key is pressed", WhenActionPressed:"when an input action is pressed",
        WhenTouched:"when the screen is touched", WhenClicked:"when I am clicked", WhenCollision:"when I touch", WhenMessage:"when I get a message", WhenAnimationEnds:"when an animation ends", WhenAnimationMarker:"when an animation reaches a marker", WhenEnterRoom:"when I enter a room",
        WhenQualityDrops:"when quality drops", SetRenderSetting:"set rendering quality", WhenUiEvent:"when an interface event occurs", WhenCloned:"when I start as a clone", WhenParticles:"when my particles spawn, die or collide", WhenUiClicked:"when an element is clicked", WhenUiChanged:"when an input is changed",
        WhenSceneStarts:"when scene starts", WhenSceneEnds:"when scene ends", WhenWeather:"when weather becomes", WhenCutsceneSignal:"when cutscene signal", WhenCutsceneEnds:"when cutscene ends", SwitchScene:"switch scene to", PlayCutscene:"play cutscene", SkipCutscene:"skip cutscene", CameraShake:"shake camera", SetTimeScale:"set time scale", Hitstop:"hitstop frames", SetLetterbox:"set letterbox", FadeScreen:"fade screen",
        BlockHeader:"block definition", Move:"move forward", GoTo:"go to", NavigateTo:"navigate to", ChangePosition:"change position",
        Glide:"glide to", TweenScale:"tween size", TweenRotation:"tween rotation", TweenColor:"tween color", StopTweens:"stop my tweens", PlayAnimation:"play animation", StopAnimation:"stop my animation", SetAnimationSpeed:"set animation speed", FireAnimationTrigger:"fire an animation trigger", SetRigSlot:"swap a rig slot", SetSlotTint:"tint a rig slot", SetIkTarget:"point a rig IK chain", SetSpriteDial:"set a sprite dial", Turn:"turn", SetRotation:"point in direction", PointTowards:"point towards", SetScale:"set size", SetBody:"set body",
        SetTrigger:"make me solid or a trigger", SetCollisionLayer:"set my collision layer", SetCollisionMask:"set my collision mask",
        ApplyImpulse:"push", AddForce:"add force", AddTorque:"add torque", ControllerMove:"move controller", SetController:"set controller", MotorAct:"motor", SetMotor:"set motor", JointAct:"joint", CastRay:"cast ray", CastBall:"cast ball", OverlapBall:"find in ball", FindClosest:"find closest", SetVelocity:"set velocity", SetGravity:"set gravity", SetDensity:"set density", SetMass:"set mass", Say:"say",
        SetVisible:"show or hide", SetColor:"set color", SetExposure:"set exposure", SetLightIntensity:"set my light brightness", SetEmissiveStrength:"set my glow", SetHdrOutput:"turn HDR output on or off", SetPeakBrightness:"set peak brightness", EnableVolume:"enable or disable a volume", SetVolumeWeight:"set a volume's weight", CaptureProbes:"capture light probes", SetShadowDistance:"set shadow distance", SetLightShadows:"turn my light's shadows on or off", SetRayTracing:"turn ray tracing on or off", SetGiBounces:"set GI bounces", SetGiSamples:"set GI samples", SetFogDensity:"set fog density", SetAurora:"set aurora", StrikeLightning:"strike lightning", SetLightningRate:"set lightning storm", SetWind:"set the wind", SetCloudDrift:"set cloud drift", SetClouds:"set clouds", SetCloudLayer:"set a cloud layer", SetWater:"set water level, chop or foam", SetTimeOfDay:"set time of day", AdvanceTime:"advance time", SetPrecipitation:"set rain or snow", BlendWeather:"blend weather to a preset", PaintTile:"paint a tile", SetParallax:"set a layer's parallax", BurstParticles:"burst particles", SetEmitterDial:"set an emitter dial", SetTrailEnabled:"start or stop my trail", SetEmitterPlaying:"start or stop my particles", PlaySound:"play sound", PlaySoundAt:"play sound at an actor", StopSound:"stop sound",
        SetSoundVolume:"set sound volume", SetSoundPitch:"set sound pitch", SetBusVolume:"set bus volume", SetComponentField:"set a component field",
        SetCameraView:"set my camera view", SetCameraPitch:"set camera pitch", SetCameraFov:"set camera fov", AttachComponent:"attach a component",
        DetachComponent:"detach a component", SetParent:"attach me to an actor", CreateClone:"create a clone", CreateActor:"create an actor",
        DeleteActor:"delete an actor", Wait:"wait", WaitUntil:"wait until", If:"if", IfElse:"if else", Repeat:"repeat", Forever:"forever",
        While:"while", EscapeLoop:"break out of the loop", ContinueLoop:"next loop iteration", Broadcast:"broadcast", StopAll:"stop everything",
        SetMouseLocked:"lock or unlock the mouse", RumbleGamepad:"rumble the gamepad", BindAction:"bind an input to an action",
        ClearActionBindings:"clear all bindings of an action", ShowPanel:"show a panel", ShowLabel:"show a label", ShowButton:"show a button",
        ShowImage:"show an image", ShowInput:"show a text input", ShowSlider:"show a slider", ShowToggle:"show a toggle",
        ShowWidget:"show a widget", ShowList:"show a scrollable list", SetUiTheme:"set the ui theme", BindUi:"bind widget to value", SetUiItems:"set items of widget to value", ScrollUi:"scroll widget to value", SetElementTheme:"set theme of widget to value", SetUiProp:"set an element property", HideElement:"hide an element",
        HideAllUi:"hide all ui", DeleteElement:"delete an element", FocusElement:"focus an input", ClearFocus:"clear the focus",
        PauseGame:"pause the game", ResumeGame:"resume the game", SetVariable:"set a variable", ChangeVariable:"change a variable",
        SaveVariable:"save a variable", ClearSavedVariable:"clear a saved variable", AddToList:"add to a list", DeleteOfList:"delete a list item",
        DeleteAllOfList:"clear a list", ShiftList:"shift a list", InsertIntoList:"insert into a list", ReplaceItemOfList:"replace a list item",
        ReverseList:"reverse a list", LoadJsonIntoList:"load JSON into a list", SetDictValue:"set a dict value", DeleteDictKey:"delete a dict key",
        DeleteAllOfDict:"clear a dict", LoadJsonIntoDict:"load JSON into a dict", CallBlock:"my block", Return:"return", PluginBlock:"plugin block", WhenPlugin:"when a plugin event occurs", PluginRead:"plugin reporter"
    })

    Component.onCompleted: {
        BlockRegistry.recordingTargets = false;
        BlockRegistry.registerRows(buildRows());
        const ops = {};
        for (const kind in sensing) {
            const s = sensing[kind];
            ops[kind] = { prefix: s.prefix, infix: s.infix, suffix: s.suffix, result: s.result, enumArg: s.enumArg };
        }
        ops.PluginRead = { layout: pluginLayout, result: pluginResult };
        BlockRegistry.registerOperators(ops);
        BlockRegistry.blockMenu = [
            { id: "copy", text: "Copy block", icon: "copy" },
            { id: "copy-stack", text: "Copy from here down", icon: "layers" },
            { id: "note", text: "Add a note", icon: "message-square" }
        ];
        BlockRegistry.canvasMenu = [{ id: "paste", text: "Paste blocks", icon: "clipboard-paste" }];
    }
}
