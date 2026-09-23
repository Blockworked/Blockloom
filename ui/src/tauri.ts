// One function per backend command. Everything the UI does to the document
// goes through here; bridge.ts applies the fresh state returned with it.
import { invoke, listen, getVersion } from './bridge';
import type {
  ActorComponentDto,
  AssetEntry,
  AtlasLayoutDto,
  BlockPieceDto,
  BlockShapeDto,
  BuildResult,
  BuildTarget,
  CameraDto,
  DictEntryDto,
  InstrPath,
  InstructionDto,
  LightingDto,
  ListItemDto,
  Mode,
  PhysicsDto,
  PipelineReportDto,
  PlacementDto,
  PostProcessDto,
  SoundMixerDto,
  StateDto,
  ValueDto,
  ValueKind,
  ValueLocation,
  VisualDto,
} from './types';

export function getState(): Promise<StateDto> {
  return invoke('get_state');
}

export function onStateUpdated(cb: (state: StateDto) => void): Promise<() => void> {
  return listen<StateDto>('state-updated', evt => cb(evt.payload));
}

export function getAppVersion(): Promise<string> {
  return getVersion();
}

// ─── Projects ───────────────────────────────────────────────────────────────
export const openProject = (path: string) => invoke<void>('open_project', { path });
export const createProject = (name: string, location: string, mode: Mode) =>
  invoke<void>('create_project', { name, location, mode });
export const closeProject = () => invoke<void>('close_project');
/** Drops a project from the Dashboard, leaving its folder alone. */
export const forgetProject = (path: string) => invoke<void>('forget_project', { path });
/** Deletes a project's folder and everything in it. */
export const deleteProject = (path: string) => invoke<void>('delete_project', { path });
export const setProjectName = (name: string) => invoke<void>('set_project_name', { name });
export const setProjectIcon = (path: string) => invoke<void>('set_project_icon', { path });
export const saveProject = () => invoke<void>('save_project');

/** Asks for a folder. Resolves to `null` if the dialog was cancelled. */
export const pickFolder = (title: string, start: string) =>
  invoke<string | null>('pick_folder', { title, start });

/** Asks which project folder to open, then opens it. */
export async function openProjectFolder(start: string): Promise<void> {
  const path = await pickFolder('Open project', start);
  if (path) await openProject(path);
}

/** Asks where to save, then exports. Resolves quietly if the dialog was
 * cancelled. */
export async function exportProject(): Promise<void> {
  const defaultName = await invoke<string>('export_file_name');
  const path = await invoke<string | null>('pick_project_file', { save: true, defaultName });
  if (path) await invoke<void>('export_project', { path });
}

export async function importProject(): Promise<void> {
  const path = await invoke<string | null>('pick_project_file', { save: false });
  if (path) await invoke<void>('import_project', { path });
}

/** Every platform a build can be made for, this machine's first. */
export const listBuildTargets = () => invoke<BuildTarget[]>('list_build_targets');

/** Builds the open project for `target` into a folder under `path`: the
 *  player, the project's pack and its assets, which runs without Blockloom.
 *  Resolves to the runnable folder and shareable archive. */
export const buildGame = (path: string, target: string, fast: boolean) =>
  invoke<BuildResult>('build_game', { path, target, fast });

// ─── The world ──────────────────────────────────────────────────────────────
export const setMode = (mode: Mode) => invoke<void>('set_mode', { mode });
export const setBackground = (color: string) => invoke<void>('set_background', { color });
export const setGravity = (gravity: [number, number, number]) => invoke<void>('set_gravity', { gravity });
export const setFixedRate = (fixedRate: number) => invoke<void>('set_fixed_rate', { fixedRate });
export const setCamera = (camera: CameraDto) => invoke<void>('set_camera', { camera });
export const setLighting = (lighting: LightingDto) => invoke<void>('set_lighting', { lighting });
export const setSoundMixer = (mixer: SoundMixerDto) => invoke<void>('set_sound_mixer', { mixer });
export const setPostProcess = (post: PostProcessDto) => invoke<void>('set_post_process', { post });

// ─── Actors ─────────────────────────────────────────────────────────────────
export const selectActor = (actorId: string) => invoke<void>('select_actor', { actorId });
export const addActor = (shape: string) => invoke<string>('add_actor', { shape });
export const duplicateActor = (actorId: string) => invoke<string>('duplicate_actor', { actorId });
export const removeActor = (actorId: string) => invoke<void>('remove_actor', { actorId });
/** Moves an actor within the list and optionally under another one: `parent`
 * is the id it hangs off afterwards (blank for the top level) and `before`
 * the level-mate it lands in front of (blank for the end). Resolves to
 * whether anything changed. */
export const moveActor = (actorId: string, parent: string, before: string) =>
  invoke<boolean>('move_actor', { actorId, parent, before });
export const renameActor = (actorId: string, name: string) => invoke<void>('rename_actor', { actorId, name });
export const addActorComponent = (actorId: string, component: ActorComponentDto) =>
  invoke<string>('add_actor_component', { actorId, component });
/** Replaces the component currently called `name` - which is how a custom
 * component gets renamed, too. */
export const setActorComponent = (actorId: string, name: string, component: ActorComponentDto) =>
  invoke<void>('set_actor_component', { actorId, name, component });
export const removeActorComponent = (actorId: string, name: string) =>
  invoke<void>('remove_actor_component', { actorId, name });
/** Makes the actor's script file from the starter template if it isn't there
 * and attaches the component naming it. Resolves to the path. */
export const createScript = (actorId: string) => invoke<string>('create_script', { actorId });
/** Compiles one actor's script and puts the result in the run log. */
export const checkScript = (actorId: string) => invoke<void>('check_script', { actorId });
export const readScript = (actorId: string) => invoke<string>('read_script', { actorId });
export const writeScript = (actorId: string, source: string) =>
  invoke<void>('write_script', { actorId, source });

/** One error or warning pinned to a line of a script. */
export interface ScriptDiagnostic {
  path: string;
  line: number;
  column: number;
  end_line: number;
  end_column: number;
  level: string;
  message: string;
}

/** Whether this machine can compile scripts, and what it would use. */
export interface ToolchainStatus {
  available: boolean;
  rustc_version: string | null;
  rustc_error: string | null;
  cargo_version: string | null;
  cargo_error: string | null;
  help: string;
}

/** One actor's script errors pinned to their lines, for inline display. */
export const scriptDiagnostics = (actorId: string) =>
  invoke<ScriptDiagnostic[]>('script_diagnostics', { actorId });
/** Whether this machine can compile scripts - missing is a status, not an error. */
export const scriptToolchain = () => invoke<ToolchainStatus>('script_toolchain');
/** Regenerates the Cargo project rust-analyzer opens for this project's scripts. */
export const syncScriptIde = () => invoke<{ scripts: number; manifest: string }>('sync_script_ide');
/** Points the user's own editor at the project folder. Resolves to the folder and what opened it. */
export const openScriptIde = () => invoke<{ path: string; openedWith: string }>('open_script_ide');
export const setActorVisual = (actorId: string, visual: VisualDto) =>
  invoke<void>('set_actor_visual', { actorId, visual });
export const setActorPlacement = (actorId: string, placement: PlacementDto) =>
  invoke<void>('set_actor_placement', { actorId, placement });
export const setActorPhysics = (actorId: string, physics: PhysicsDto) =>
  invoke<void>('set_actor_physics', { actorId, physics });
export const setActorVisible = (actorId: string, visible: boolean) =>
  invoke<void>('set_actor_visible', { actorId, visible });

// ─── Assets ─────────────────────────────────────────────────────────────────
// A project folder's files, which the asset tray manages. None of these touch
// the document, so none of them are undoable; the tray re-lists after each.

/** What one folder of the project holds. `path` is `''` for the top. */
export const listAssets = (path: string) => invoke<AssetEntry[]>('list_assets', { path });
export const createAssetFolder = (parent: string, name: string) =>
  invoke<string>('create_asset_folder', { parent, name });
/** Makes an empty file - a script gets the starter template. */
export const createAsset = (parent: string, name: string) =>
  invoke<string>('create_asset', { parent, name });
export const importAssets = (parent: string, paths: string[]) =>
  invoke<string[]>('import_assets', { parent, paths });
/** Renaming and moving both follow the asset through the document, so an
 * actor using it doesn't end up pointing at nothing. */
export const renameAsset = (path: string, name: string) =>
  invoke<string>('rename_asset', { path, name });
export const moveAsset = (path: string, parent: string) =>
  invoke<string>('move_asset', { path, parent });
export const deleteAsset = (path: string) => invoke<void>('delete_asset', { path });
/** A file's bytes as a `data:` URL - the only way the page can show one. */
export const readAsset = (path: string) => invoke<string>('read_asset', { path });
/** Pops the native file manager open on the folder this asset lives in. */
export const openAssetLocation = (path: string) => invoke<void>('open_asset_location', { path });
/** What the pipeline makes of one asset: rig counts, texture/audio plan, dirt. */
export const inspectAsset = (path: string) => invoke<PipelineReportDto>('inspect_asset', { path });
/** Every asset with its pipeline report and reimport dirt. */
export const pipelineStatus = () => invoke<PipelineReportDto[]>('pipeline_status');
/** Re-inspects assets and refreshes fingerprints; empty paths means everything dirty. */
export const reimportAssets = (paths: string[]) =>
  invoke<PipelineReportDto[]>('reimport_assets', { paths });
/** Lays images into one atlas sheet plan without writing files. */
export const packAtlas = (paths: string[], maxSize?: number, padding?: number) =>
  invoke<AtlasLayoutDto>('pack_atlas', { paths, maxSize, padding });

/** Asks which files to import. Resolves to `null` if the dialog was
 * cancelled. */
export const pickFiles = (title: string) => invoke<string[] | null>('pick_files', { title });

// ─── Running ────────────────────────────────────────────────────────────────
export const runProject = () => invoke<void>('run_project');
export const stopProject = () => invoke<void>('stop_project');
export const pauseProject = (paused: boolean) => invoke<void>('pause_project', { paused });
export const stepProject = () => invoke<void>('step_project');
export const closeRuntime = () => invoke<void>('close_runtime');

// ─── Embedded preview ───────────────────────────────────────────────────────
// The viewport reads MJPEG straight from the runtime's loopback sidecar
// (`http://127.0.0.1:{port}/preview.mjpg`), so frames never cross invoke.
// These commands only switch the sidecar, resize the stream and forward
// input; the port arrives on state as `preview_port`.
export const setPreviewEnabled = (enabled: boolean) =>
  invoke<void>('set_preview_enabled', { enabled });
export const setPreviewHeadless = (headless: boolean) =>
  invoke<void>('set_preview_headless', { headless });
export const setPreviewSize = (width: number, height: number) =>
  invoke<void>('set_preview_size', { width, height });
export type PreviewInputDto =
  | { kind: 'mouse_move'; x: number; y: number; w: number; h: number }
  | { kind: 'mouse_button'; button: number; down: boolean; x: number; y: number; w: number; h: number }
  | { kind: 'key'; code: string; down: boolean }
  | { kind: 'text'; text: string };
export const previewInput = (input: PreviewInputDto) =>
  invoke<void>('preview_input', { input });

// ─── Instructions ───────────────────────────────────────────────────────────
export const addInstruction = (strandId: string, path: InstrPath, instruction: InstructionDto) =>
  invoke<void>('add_instruction', { strandId, path, instruction });
export const editInstruction = (strandId: string, path: InstrPath, instruction: InstructionDto) =>
  invoke<void>('edit_instruction', { strandId, path, instruction });
export const removeInstruction = (strandId: string, path: InstrPath) =>
  invoke<void>('remove_instruction', { strandId, path });
export const deleteInstruction = (strandId: string, path: InstrPath, x: number, y: number) =>
  invoke<string | null>('delete_instruction', { strandId, path, x, y });
export const pasteInstructions = (x: number, y: number, instructions: InstructionDto[]) =>
  invoke<string>('paste_instructions', { x, y, instructions });

// ─── Strands ────────────────────────────────────────────────────────────────
export const addStrand = (x: number | null, y: number | null, instruction: InstructionDto | null) =>
  invoke<string>('add_strand', { x, y, instruction });
export const removeStrand = (strandId: string) => invoke<void>('remove_strand', { strandId });
export const moveStrand = (strandId: string, x: number, y: number) =>
  invoke<void>('move_strand', { strandId, x, y });
export const splitStrand = (strandId: string, path: InstrPath, x: number, y: number) =>
  invoke<string>('split_strand', { strandId, path, x, y });
export const mergeStrand = (draggedId: string, targetId: string, path: InstrPath) =>
  invoke<void>('merge_strand', { draggedId, targetId, path });

// ─── Values ─────────────────────────────────────────────────────────────────
export const editValueField = (location: ValueLocation, text: string) =>
  invoke<void>('edit_value_field', { location, text });
export const setValueKind = (location: ValueLocation, kind: ValueKind) =>
  invoke<void>('set_value_kind', { location, kind });
export const takeValue = (location: ValueLocation) => invoke<ValueDto>('take_value', { location });
export const putValue = (location: ValueLocation, value: ValueDto) =>
  invoke<void>('put_value', { location, value });
export const previewValue = (value: ValueDto) => invoke<string>('preview_value', { value });
export const createFloatingValue = (x: number, y: number, value: ValueDto, originBlockId: string | null) =>
  invoke<string>('create_floating_value', { x, y, value, originBlockId });
export const moveFloatingValue = (floatingId: string, x: number, y: number) =>
  invoke<void>('move_floating_value', { floatingId, x, y });
export const removeFloatingValue = (floatingId: string) =>
  invoke<void>('remove_floating_value', { floatingId });

// ─── Comments ───────────────────────────────────────────────────────────────
export const createComment = (x: number, y: number, text: string) =>
  invoke<string>('create_comment', { x, y, text });
export const createAttachedComment = (attachedTo: string, x: number, y: number, text: string) =>
  invoke<string>('create_comment', { x, y, text, attachedTo });
export const moveComment = (commentId: string, x: number, y: number) =>
  invoke<void>('move_comment', { commentId, x, y });
export const editCommentText = (commentId: string, text: string) =>
  invoke<void>('edit_comment_text', { commentId, text });
export const setCommentCollapsed = (commentId: string, collapsed: boolean) =>
  invoke<void>('set_comment_collapsed', { commentId, collapsed });
export const removeComment = (commentId: string) => invoke<void>('remove_comment', { commentId });

// ─── Variables and custom blocks ────────────────────────────────────────────
export const createVariable = (name: string, scope: 'actor' | 'global') =>
  invoke<void>('create_variable', { name, scope });
export const renameVariable = (oldName: string, newName: string) =>
  invoke<void>('rename_variable', { oldName, newName });
export const deleteVariable = (name: string) => invoke<void>('delete_variable', { name });
// ─── Input actions ────────────────────────────────────────────────────────────
export const createInputAction = (name: string) => invoke<void>('create_input_action', { name });
export const renameInputAction = (oldName: string, newName: string) =>
  invoke<void>('rename_input_action', { oldName, newName });
export const deleteInputAction = (name: string) => invoke<void>('delete_input_action', { name });
export const addInputBinding = (name: string, binding: string) =>
  invoke<boolean>('add_input_binding', { name, binding });
export const removeInputBinding = (name: string, binding: string) =>
  invoke<boolean>('remove_input_binding', { name, binding });
export const clearInputBindings = (name: string) => invoke<boolean>('clear_input_bindings', { name });
// A list item is a literal number or text - see `ListItemDto` in types.ts.
export const createList = (name: string, scope: 'actor' | 'global') =>
  invoke<void>('create_list', { name, scope });
export const renameList = (oldName: string, newName: string) =>
  invoke<void>('rename_list', { oldName, newName });
export const deleteList = (name: string) => invoke<void>('delete_list', { name });
export const setListItems = (name: string, items: ListItemDto[]) =>
  invoke<void>('set_list_items', { name, items });
export const setListEditorState = (name: string, visible: boolean, x: number, y: number) =>
  invoke<void>('set_list_editor_state', { name, visible, x, y });

// A dict entry is a key with a literal number or text value - see `DictEntryDto`.
export const createDict = (name: string, scope: 'actor' | 'global') =>
  invoke<void>('create_dict', { name, scope });
export const renameDict = (oldName: string, newName: string) =>
  invoke<void>('rename_dict', { oldName, newName });
export const deleteDict = (name: string) => invoke<void>('delete_dict', { name });
export const setDictEntries = (name: string, entries: DictEntryDto[]) =>
  invoke<void>('set_dict_entries', { name, entries });
export const setDictEditorState = (name: string, visible: boolean, x: number, y: number) =>
  invoke<void>('set_dict_editor_state', { name, visible, x, y });
export const createBlock = (pieces: BlockPieceDto[], shape: BlockShapeDto, color: string) =>
  invoke<string>('create_block', { pieces, shape, color });
export const editBlock = (blockId: string, pieces: BlockPieceDto[], shape: BlockShapeDto, color: string) =>
  invoke<void>('edit_block', { blockId, pieces, shape, color });
export const deleteBlock = (blockId: string) => invoke<void>('delete_block', { blockId });

// ─── Undo and the log ───────────────────────────────────────────────────────
export const undo = () => invoke<void>('undo');
export const redo = () => invoke<void>('redo');
export const clearLog = () => invoke<void>('clear_log');
export const pushLog = (kind: string, text: string) => invoke<void>('push_log', { kind, text });

// ─── The window ─────────────────────────────────────────────────────────────
/** Resets page zoom to 100% - Ctrl/Cmd+0 can't rely on the browser's own
 * accelerator in this CEF runtime. */
export const resetZoom = () => invoke<void>('reset_zoom');
/** Matches the native window's background to the theme, so no white shows
 * before the page paints or while the window closes. */
export const setThemeBackground = (theme: 'light' | 'dark') =>
  invoke<void>('set_theme_background', { theme });
