// The actor list's own state: which actor is mid-drag for a move, and
// which parents show collapsed. Only per-eye settings live here - nothing
// the project owns. Mirrors `assets.ts`, including its CEF workaround: this
// runtime never delivers the `drop` half of an in-page drag, so an
// unconsumed `dragend` re-runs the drop at its release point instead.
import { reactive, ref } from 'vue';
import { actorParent, type ActorDto } from './types';

/** The drag payload's own type, so an actor dropped anywhere else is ignored. */
export const ACTOR_MIME = 'application/x-blockloom-actor';

/** The actor under the cursor right now, while one is being dragged. */
export const draggedActor = ref<string | null>(null);

export function startActorDrag(e: DragEvent, actorId: string): void {
  draggedActor.value = actorId;
  if (!e.dataTransfer) return;
  e.dataTransfer.effectAllowed = 'move';
  e.dataTransfer.setData(ACTOR_MIME, actorId);
  e.dataTransfer.setData('text/plain', actorId);
}

export function endActorDrag(): void {
  draggedActor.value = null;
}

/** The actor a drop is carrying, or null if it isn't carrying one. */
export function droppedActor(e: DragEvent): string | null {
  const raw = e.dataTransfer?.getData(ACTOR_MIME);
  if (raw) return raw;
  // CEF hides custom types from a same-page drop often enough that the
  // mirrored id is the reliable answer; the dataTransfer is the fallback.
  return draggedActor.value;
}

// See `assets.ts`: without this the release over a row or the unparent zone
// lands nowhere in the editor window, while the dev-bridge browser tab fires
// a real `drop` first and this listener no-ops (the drop already cleared it).
window.addEventListener(
  'dragend',
  e => {
    if (!draggedActor.value) return;
    const el = document
      .elementFromPoint(e.clientX, e.clientY)
      ?.closest('[data-actor-drop]');
    if (!el) return;
    el.dispatchEvent(
      new DragEvent('drop', {
        bubbles: true,
        cancelable: true,
        clientX: e.clientX,
        clientY: e.clientY,
        dataTransfer: e.dataTransfer ?? null,
      }),
    );
  },
  true,
);

// ─── Collapsed parents ──────────────────────────────────────────────────────

const COLLAPSED_KEY = 'blockloom.actors.collapsed';

function readCollapsed(): string[] {
  try {
    const raw = window.localStorage.getItem(COLLAPSED_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === 'string') : [];
  } catch {
    // A page with storage blocked still works; it just forgets.
    return [];
  }
}

function persist(collapsed: Set<string>): void {
  try {
    window.localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed]));
  } catch {
    // As above.
  }
}

/** The parents whose children are hidden. Stale ids (deleted actors, actors
 * that lost their children) are harmless: nothing renders a toggle for them.
 * Keyed by actor id, which is stable across renames. */
export const collapsedParents = reactive<Set<string>>(new Set(readCollapsed()));

export function isCollapsed(actorId: string): boolean {
  return collapsedParents.has(actorId);
}

export function toggleCollapsed(actorId: string): void {
  if (collapsedParents.has(actorId)) collapsedParents.delete(actorId);
  else collapsedParents.add(actorId);
  persist(collapsedParents);
}

export function expandParent(actorId: string): void {
  if (collapsedParents.delete(actorId)) persist(collapsedParents);
}

// ─── The tree ───────────────────────────────────────────────────────────────

/** Whether `id` already hangs off `ancestorId`, directly or further down -
 * hanging `ancestorId` off `id` would make a loop. */
export function hangsOff(actors: ActorDto[], id: string, ancestorId: string): boolean {
  const seen = new Set<string>();
  let at: string | null = id;
  while (at && !seen.has(at)) {
    seen.add(at);
    at = actorParent(actors.find(candidate => candidate.id === at) ?? null);
    if (at === ancestorId) return true;
  }
  return false;
}

/** Whether dropping `childId` onto `parentId` would make a loop: onto itself
 * or onto something already hanging off it. The backend refuses the same
 * drops in `check_parent`; this is the list's upfront answer. */
export function wouldCycle(actors: ActorDto[], childId: string, parentId: string): boolean {
  return parentId === childId || hangsOff(actors, parentId, childId);
}

/** The actors with no parent - or whose parent is gone - in project order. */
export function rootActors(actors: ActorDto[]): ActorDto[] {
  const known = new Set(actors.map(actor => actor.id));
  return actors.filter(actor => {
    const parent = actorParent(actor);
    return !parent || !known.has(parent);
  });
}

/** One parent's children, in project order. */
export function childrenOf(actors: ActorDto[], parentId: string): ActorDto[] {
  return actors.filter(actor => actorParent(actor) === parentId);
}
