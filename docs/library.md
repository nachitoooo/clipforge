# Library (folders)

The **Biblioteca** tab holds the folders a person makes to organize clips.
They are created, renamed in place, edited (name, color, icon) and deleted from
the UI; a clip can be in **several folders at once**.

## Using it

* **Make one:** *Nueva carpeta* (name, color, icon) — names are unique without
  regard to capitals. Anything goes in a name except control characters (it is
  a label, not a file name), up to 60 characters.
* **Put clips in:** on any clip card (Recientes, Biblioteca, inside a folder)
  the *Carpetas…* button opens a list with a checkbox per folder and a
  "Nueva carpeta…" row. Or drag a clip from *Sin carpeta* onto a folder card:
  dropping **adds** it (it never moves, a clip can be in many folders).
* **Look inside:** click a folder. *Sacar de esta carpeta* takes a clip out
  without deleting it.
* **Rename / edit:** the pencil on a folder card renames it in place; the
  palette opens the full dialog; both are also in the folder's own header.
* **Delete:** a folder with no clips just asks. With clips, the dialog offers
  *Conservar los clips* (they stay in Recientes) or *Borrar carpeta y N clips*:
  only the clips that are in **no other folder** are deleted from the disk; a
  clip that is also filed elsewhere stays, it only leaves this folder.
* **Recientes** keeps showing every clip; each card shows the folders it is in
  (two labels and a "+N").

## How it is stored

A folder is a label over the flat clips folder — nothing moves on disk, so
`describe_clip`, `trim_clip`, the player and the editor still name a clip by its
file name. The index is `library.json` in the app config folder, next to
`settings.json`:

```json
{ "version": 1, "nextId": 3,
  "folders": [ { "id": "f1", "name": "Valorant", "color": "red", "icon": "target",
                 "clips": ["Ace en Mirage.mp4"], "createdMs": 1760000000000 } ] }
```

* `id` (`f1`, `f2`…) is stable and never reused, so renaming never breaks
  membership. `color` and `icon` are short ids (`[a-z-]`, ≤ 16); the colors and
  drawings live in the UI (`lib/folders.ts`, `ui/icons.ts`) and an id it doesn't
  know falls back to the default, so a newer file still opens in an older app.
* Everything has a serde default; a hand-edited file is tidied on load
  (duplicate ids, empty names, an id counter that went backwards).
* An unreadable file is moved to `library.json.bak` and the library starts empty.
* Writes are atomic (`persist.rs`) and all or nothing: memory only changes if
  the file was written, and a change that changes nothing doesn't write.

## Rules that keep the index honest

* **Reads never write and never prune.** A name whose file is missing (deleted
  in the Finder, a drive that is unplugged) is inert: it isn't counted or shown,
  but it isn't forgotten either, so a transient problem can't empty the index.
* The app's own actions keep it in step (`lib.rs`): *rename clip* → the folders
  follow the new name; *delete clip* → it leaves every folder; *trim clip* → the
  `-recorte` copy joins the same folders as the original. If writing the index
  fails after the file operation succeeded, it is logged and the file operation
  is not undone.
* The webview only sends clip **file names** and folder **ids**; the backend
  checks names with `library::clip_path` and that the clips exist before filing.

## Code map

| Piece | Where |
|---|---|
| Index, validation, store | `app/src-tauri/src/folders.rs` |
| Delete folder (+ its exclusive clips) | `library::delete_folder` |
| Commands + events (`library-changed`) | `app/src-tauri/src/lib.rs` |
| Pure UI logic (name check, palette, sorting, delete breakdown) | `app/src/lib/folders.ts` |
| Views | `ui/library.ts`, `ui/clipcard.ts` (shared card), `ui/folderdialog.ts`, `ui/folderpicker.ts` |

Every mutation emits `library-changed`; Recientes and Biblioteca both reload on
it (and on `clip-saved`, and when the window regains focus).

## Smart library (F-3.10)

Searching, filtering and sorting over Recientes, the Biblioteca root and the
inside of each folder, and a **game** on every clip.

* **The bar** (`ui/filterbar.ts`, pure logic in `lib/filter.ts`): search box,
  game, folder (not inside a folder), date (today / 7 days / 30 days / this year)
  and order (newest, oldest, name, size). The search ignores capitals and accents,
  every word typed has to match, and it looks at the title, the game **and the
  names of the clip's folders** — the folders are the tags (a clip can be in
  several). Each view keeps its own filter in memory; the Biblioteca keeps one
  for the root and a fresh one each time a folder is opened.
* **In the Biblioteca** a filter turns the root into a list of results over every
  clip, folder or not (plus the folders whose name matches a search that is
  not narrowed by anything else). Without a filter it is the folders and
  «Sin carpeta», and the order still applies to the latter.
* **Cards are kept** (`clipcard.ts`) while nothing they show changed, so typing in
  the search box doesn't reload every thumbnail.

### Where the game comes from

While the buffer runs, `core/src/activity/` looks once a second at which
application is in front (`NSWorkspace` on macOS; the foreground window's process on
Windows, compiled but never run). Only the application's **name** is read, never
window titles or contents, and it stays on this machine. When a clip is saved,
`ActivityLog::dominant` picks the application that held the screen the longest
during the clip's span — it has to hold at least a fifth of it — ignoring ClipForge
itself and the desktop shells (`default_ignore`), so saving from the tray or the
window still names the game. Nothing stood out → no game («Sin juego»).

The game is stored per clip in `clipmeta.json` (`app/src-tauri/src/clipmeta.rs`),
next to `library.json`, under the same rules: keyed by file name, reads never write
or prune, all-or-nothing writes, kept in step when a clip is renamed, deleted or
trimmed (the copy keeps the game). Both indexes share `persist::JsonStore`.

* **Correct it:** the game on a card is a button. «Juego del clip» suggests the
  games already in the library and, when other clips have the same game, offers
  «Cambiar en todos los clips de “X”» — the OS names can be ugly
  (`VALORANT-Win64-Shipping`). The same game with other capitals is one game (the
  first spelling wins).
* **Switch it off:** Configuración → Biblioteca → «Detectar el juego al guardar
  un clip» (on by default). Turning it off stops the lookup and forgets what it
  saw; clips already tagged keep their game.

### Limits

* Clips saved before this existed have no game until it is set by hand.
* A game that is minimized or in another window while the clip is recorded can be
  missed; a game running as administrator on Windows refuses the process lookup.
* On Windows the lookup has been compiled and linted by the CI cross-check, never
  run (F-2.4 is open).

## Known limits

* Renaming or deleting a clip **outside** the app (Finder/Explorer) takes it
  out of its folders (the file is just a different name now); the entry stays
  inert in `library.json` and comes back if the same name reappears.
* Changing the clips folder (a planned setting) leaves the memberships inert:
  they are keyed by file name, not by path.
* Folders are one level deep and sorted by name; there is no manual order,
  nesting, bulk select or cover thumbnail yet (see the backlog).
* Drag and drop: `tauri.conf.json` sets `dragDropEnabled: false` because on
  Windows (WebView2) Tauri's file-drop handler swallows HTML5 drag and drop.
  The app doesn't take files dropped from the OS. Not yet tried on Windows.

## Adding a color or an icon

A color is a `{ id, label, hex }` in `FOLDER_COLORS`; an icon is an id in
`FOLDER_ICONS` plus its drawing in `folderIcons` (`ui/icons.ts`). Rust accepts
any short lowercase id, so nothing else changes. `lib/folders.test.ts` checks
that every id is one Rust accepts.
