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
    // Option lists are functions of the state, so re-evaluate them with it.
    onAppStateChanged: BlockRegistry.revision++

    // ─── Fixed option lists (constants.ts) ─────────────────────────────────
    function opts(names) { return names.map(n => ({ value: n, label: n })); }
    readonly property var keyOptions: opts(["space","up arrow","down arrow","left arrow","right arrow","enter","escape","shift","control","alt","tab","backspace"]
        .concat("abcdefghijklmnopqrstuvwxyz".split("")).concat("0123456789".split("")))
    // The atmosphere slot's readings, spelled as `AtmosphereSense::field` takes them.
    readonly property var atmosphereOptions: opts(["sun x","sun y","sun z","sun brightness","wind x","wind y","wind z","wind speed","wind gust","fog density","cloud cover","rain","snow","wetness","temperature","exposure"])
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
    readonly property var uiThemeOptions: [{value:"Dark",label:"dark"},{value:"Light",label:"light"},{value:"HighContrast",label:"high contrast"}]
    readonly property var soundBusOptions: [{value:"Master",label:"master"},{value:"Music",label:"music"},{value:"Sfx",label:"sound effects"}]
    readonly property var soundLoopOptions: [{value:"false",label:"once"},{value:"true",label:"loop"}]
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
        const extra = ["Look","Render","Body","Joint","Brain","Camera","Material","Emitter","Trail","Light"].filter(n => held.indexOf(n) < 0);
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
        WhenStarted:"flag", WhenKeyPressed:"keyboard", WhenActionPressed:"gamepad-2", WhenTouched:"pointer", WhenClicked:"mouse-pointer-click",
        WhenCollision:"crosshair", WhenMessage:"radio", WhenCloned:"copy", WhenUiEvent:"square-mouse-pointer", WhenUiClicked:"square-mouse-pointer", WhenUiChanged:"sliders-horizontal",
        BlockHeader:"blocks", Move:"arrow-right", GoTo:"move", NavigateTo:"navigation", ChangePosition:"move-3d", Glide:"wind", Turn:"rotate-cw",
        SetRotation:"rotate-cw", PointTowards:"target", SetScale:"maximize", SetBody:"boxes", SetTrigger:"ghost", SetCollisionLayer:"layers",
        SetCollisionMask:"filter", ApplyImpulse:"zap", SetVelocity:"trending-up", SetGravity:"cloud", SetDensity:"weight", SetMass:"weight",
        Say:"message-square", SetVisible:"eye", SetColor:"palette", SetExposure:"sun", SetLightIntensity:"zap", BurstParticles:"sparkles", SetEmitterDial:"sliders-horizontal", SetTrailEnabled:"wind", PlaySound:"volume-2", PlaySoundAt:"map-pin", StopSound:"square",
        SetSoundVolume:"volume-1", SetSoundPitch:"music", SetBusVolume:"sliders-horizontal", SetComponentField:"panels-top-left",
        SetCameraView:"camera", SetCameraPitch:"video", SetCameraFov:"video", AttachComponent:"plus", DetachComponent:"x", SetParent:"link",
        CreateClone:"copy", CreateActor:"sparkles", DeleteActor:"trash-2", Wait:"clock", WaitUntil:"hand", If:"git-branch", IfElse:"git-fork",
        Repeat:"repeat", Forever:"infinity", While:"repeat", EscapeLoop:"log-out", ContinueLoop:"skip-forward", Broadcast:"radio", StopAll:"octagon",
        SetMouseLocked:"lock", RumbleGamepad:"vibrate", BindAction:"keyboard", ClearActionBindings:"eraser", ShowPanel:"layout-panel-top",
        ShowLabel:"type", ShowButton:"square-mouse-pointer", ShowImage:"image", ShowInput:"text-cursor-input", ShowSlider:"sliders-horizontal",
        ShowWidget:"layout-grid", ShowToggle:"toggle-left", ShowList:"list-checks", SetUiTheme:"palette", BindUi:"list-checks", SetUiItems:"list-checks", ScrollUi:"list-checks", SetElementTheme:"list-checks", SetUiProp:"list-checks", HideElement:"eye", HideAllUi:"eye",
        DeleteElement:"trash-2", FocusElement:"text-select", ClearFocus:"text-select", PauseGame:"pause", ResumeGame:"play", SetVariable:"asterisk",
        ChangeVariable:"trending-up", SaveVariable:"save", ClearSavedVariable:"trash-2", AddToList:"plus", DeleteOfList:"trash-2",
        DeleteAllOfList:"trash-2", ShiftList:"arrow-left", InsertIntoList:"plus", ReplaceItemOfList:"repeat", ReverseList:"rotate-cw",
        LoadJsonIntoList:"braces", SetDictValue:"book-plus", DeleteDictKey:"trash-2", DeleteAllOfDict:"trash-2", LoadJsonIntoDict:"braces",
        CallBlock:"blocks", Return:"corner-down-right"
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
            WhenCollision: header([lb("when I touch"), dd("with", () => [{ value: "", label: "anything" }].concat(actorOptions(false)), { placeholder: "anything" })]),
            WhenMessage: header([lb("when I get"), field("name", "message")]),
            WhenCloned: header([lb("when I start as a clone")]),
            WhenUiEvent: header([lb("when"), field("element", "element id"), dd("event", () => opts(["press", "release", "hover", "leave", "drag", "scroll", "focus"]))]),
            WhenUiClicked: header([lb("when"), field("element", "element id"), lb("clicked")]),
            WhenUiChanged: header([lb("when"), field("element", "element id"), lb("changed")]),
            Broadcast: row([lb("broadcast"), field("name", "message")]),
            // Motion
            Move: row([lb("move"), slot("MoveSteps", "steps"), lb("steps")]),
            GoTo: row(vector("go to", ["GoToX","GoToY","GoToZ"], ["x","y","z"])),
            NavigateTo: row(vector("navigate to", ["NavigateX","NavigateY","NavigateZ"], ["x","y","z"]).concat([lb("at speed"), slot("NavigateSpeed", "speed")])),
            ChangePosition: row([lb("change"), dd("axis", positionAxes), lb("by"), slot("ChangeByAmount", "by")]),
            Glide: row([lb("glide"), slot("GlideSeconds", "seconds"), lb("secs to x:"), slot("GlideX", "x"), lb("y:"), slot("GlideY", "y"), lb("z:", true), slot("GlideZ", "z", { when: is3d })]),
            Turn: row([lb("turn"), dd("axis", rotationAxes, { when: is3d }), lb("by"), slot("TurnDegrees", "degrees"), lb("degrees")]),
            SetRotation: row([lb("point"), dd("axis", rotationAxes, { when: is3d }), lb("in direction"), slot("RotationDegrees", "degrees")]),
            PointTowards: row([lb("point towards"), dd("target", () => actorOptions(true))]),
            SetScale: row([lb("set size to"), slot("ScaleFactor", "factor")]),
            // Physics
            SetBody: row([lb("set body to"), dd("body", bodyOptions)]),
            ApplyImpulse: row(vector("push", ["ImpulseX","ImpulseY","ImpulseZ"], ["x","y","z"])),
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
            SetExposure: row([lb("set exposure to"), slot("ExposureEv", "ev"), lb("EV")]),
            SetLightIntensity: row([lb("set my light to"), slot("LightIntensity", "intensity"), lb("lumens")]),
            BurstParticles: row([lb("burst"), slot("ParticleCount", "count"), lb("particles")]),
            SetEmitterDial: row([lb("set emitter"), dd("dial", () => opts(["Rate","Lifetime","Speed","Spread","Gravity","SizeStart","SizeEnd","Max"])), lb("to"), slot("EmitterValue", "value")]),
            SetTrailEnabled: row([flagDefaultTrue("enabled", visibleOptions), lb("my trail")]),
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
            Return: cap([lb("return"), slot("ReturnValue", "value")])
        };
        for (const type in r) r[type].icon = icons[type] || "blocks";
        return r;
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
        case "WhenCollision": return { with: "" };
        case "WhenMessage": case "Broadcast": return { name: "message1" };
        case "BlockHeader": return { block_id: "" };
        case "Move": return { steps: num(10) };
        case "GoTo": return { x: num(0), y: num(0), z: num(0) };
        case "NavigateTo": return { x: num(0), y: num(0), z: num(0), speed: num(4) };
        case "ChangePosition": return { axis: "X", by: num(10) };
        case "Glide": return { seconds: num(1), x: num(0), y: num(0), z: num(0) };
        case "Turn": return { axis: "Z", degrees: num(15) };
        case "SetRotation": return { axis: "Z", degrees: num(0) };
        case "PointTowards": return { target: "mouse" };
        case "SetScale": return { factor: num(1) };
        case "SetBody": return { body: "Dynamic" };
        case "ApplyImpulse": return { x: num(0), y: num(5), z: num(0) };
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
        case "SetExposure": return { ev: num(9.7) };
        case "SetLightIntensity": return { intensity: num(800) };
        case "BurstParticles": return { count: num(24) };
        case "SetEmitterDial": return { dial: "Rate", value: num(24) };
        case "SetTrailEnabled": return { enabled: true };
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
        SoundPlaying: { prefix: "is", suffix: "playing?", result: "bool", arity: 1, args: ["text"] },
        BusVolume: { prefix: "volume of", suffix: "bus", result: "number", enumArg: { index: 0, options: soundBusOptions }, arity: 1, args: ["text"] },
        Atmosphere: { prefix: "air", result: "number", enumArg: { index: 0, options: atmosphereOptions }, arity: 1, args: ["text"] }
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
        { label: "Sensing", kinds: ["KeyDown","MouseDown","MouseButtonDown","MouseX","MouseY","MouseDeltaX","MouseDeltaY","MouseLocked","ActionDown","ActionPressed","ActionReleased","ActionValue","TouchCount","TouchX","TouchY","GamepadConnected","GamepadAxis","GamepadButtonDown","Timer","MyPosition","MyRotation","MyLocalPosition","Touching","DistanceTo","IsTrigger","CollisionLayer","RayHit","RayDistance","CircleHit","ActorPosition","ActorLocalPosition","ComponentField","SoundPlaying","BusVolume","Atmosphere"] },
        { label: "Interface", kinds: ["UiSelectedIndex","UiValue","UiText","UiShown","UiExists","UiFocus","GamePaused"] },
        { label: "Actors", kinds: ["IsClone","MyParent","NewActor","ActorCount"] },
        { label: "Maths", kinds: ["Add","Sub","Mul","Div","Mod","Round","Math","Random"] },
        { label: "Comparing", kinds: ["Eq","Neq","Gt","Lt","Gte","Lte","And","Or","Not","True","False"] },
        { label: "Text", kinds: ["Join","Join3","Length","LetterOf","IndexOf","LastIndexOf","Case","NewLine","Tab","CurrentTime"] }
    ]
    readonly property var listOperatorKinds: ["ListItem","ListItemNumber","ListAmount","ListLength","ListContains","ListItemExists","ListIsEmpty","ListAsJson"]
    readonly property var dictOperatorKinds: ["DictValue","DictHasKey","DictSize","DictKeys","DictAsJson","DictIsEmpty"]
    readonly property var blockGroups: [
        { label: "Events", types: ["WhenStarted","WhenKeyPressed","WhenActionPressed","WhenTouched","WhenClicked","WhenCollision","WhenMessage","WhenCloned","WhenUiEvent","WhenUiClicked","WhenUiChanged","Broadcast"] },
        { label: "Motion", types: ["Move","GoTo","NavigateTo","ChangePosition","Glide","Turn","SetRotation","PointTowards","SetScale"] },
        { label: "Physics", types: ["SetBody","ApplyImpulse","SetVelocity","SetGravity","SetDensity","SetMass","SetTrigger","SetCollisionLayer","SetCollisionMask"] },
        { label: "Looks", types: ["Say","SetVisible","SetColor","SetExposure","SetLightIntensity","BurstParticles","SetEmitterDial","SetTrailEnabled"] },
        { label: "Sound", types: ["PlaySound","PlaySoundAt","StopSound","SetSoundVolume","SetSoundPitch","SetBusVolume"] },
        { label: "Components", types: ["SetComponentField","SetCameraView","SetCameraPitch","SetCameraFov","AttachComponent","DetachComponent","SetParent"] },
        { label: "Actors", types: ["CreateClone","CreateActor","DeleteActor"] },
        { label: "Interface", types: ["ShowWidget","ShowPanel","ShowLabel","ShowButton","ShowImage","ShowInput","ShowSlider","ShowToggle","ShowList","SetUiTheme","BindUi","SetUiItems","ScrollUi","SetElementTheme","SetUiProp","HideElement","HideAllUi","DeleteElement","FocusElement","ClearFocus","PauseGame","ResumeGame"] },
        { label: "Control", types: ["Wait","WaitUntil","If","IfElse","Repeat","Forever","While","EscapeLoop","ContinueLoop","StopAll","SetMouseLocked"] },
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
        WhenStarted:"when the project starts", WhenKeyPressed:"when a key is pressed", WhenActionPressed:"when an input action is pressed",
        WhenTouched:"when the screen is touched", WhenClicked:"when I am clicked", WhenCollision:"when I touch", WhenMessage:"when I get a message",
        WhenUiEvent:"when an interface event occurs", WhenCloned:"when I start as a clone", WhenUiClicked:"when an element is clicked", WhenUiChanged:"when an input is changed",
        BlockHeader:"block definition", Move:"move forward", GoTo:"go to", NavigateTo:"navigate to", ChangePosition:"change position",
        Glide:"glide to", Turn:"turn", SetRotation:"point in direction", PointTowards:"point towards", SetScale:"set size", SetBody:"set body",
        SetTrigger:"make me solid or a trigger", SetCollisionLayer:"set my collision layer", SetCollisionMask:"set my collision mask",
        ApplyImpulse:"push", SetVelocity:"set velocity", SetGravity:"set gravity", SetDensity:"set density", SetMass:"set mass", Say:"say",
        SetVisible:"show or hide", SetColor:"set color", SetExposure:"set exposure", SetLightIntensity:"set my light brightness", BurstParticles:"burst particles", SetEmitterDial:"set an emitter dial", SetTrailEnabled:"start or stop my trail", PlaySound:"play sound", PlaySoundAt:"play sound at an actor", StopSound:"stop sound",
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
        DeleteAllOfDict:"clear a dict", LoadJsonIntoDict:"load JSON into a dict", CallBlock:"my block", Return:"return"
    })

    Component.onCompleted: {
        BlockRegistry.recordingTargets = false;
        BlockRegistry.registerRows(buildRows());
        const ops = {};
        for (const kind in sensing) {
            const s = sensing[kind];
            ops[kind] = { prefix: s.prefix, infix: s.infix, suffix: s.suffix, result: s.result, enumArg: s.enumArg };
        }
        BlockRegistry.registerOperators(ops);
        BlockRegistry.blockMenu = [
            { id: "copy", text: "Copy block", icon: "copy" },
            { id: "copy-stack", text: "Copy from here down", icon: "layers" },
            { id: "note", text: "Add a note", icon: "message-square" }
        ];
        BlockRegistry.canvasMenu = [{ id: "paste", text: "Paste blocks", icon: "clipboard-paste" }];
    }
}
