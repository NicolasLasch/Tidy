# README media — shot list

Everything the README references lives here. Use the **large mock** so no personal files appear:
`cargo run --example make_mock -p tidy-agent-runtime -- ~/TidyMock --large`, then add `~/TidyMock/Inbox` (organize demos) and `~/TidyMock` (Storage / chat demos) as folders in Tidy.

Window size for stills: **1440×900**, capture at 2× (⌘⇧4, then Space, click the window; hold ⌥ to drop the shadow). Use **dark mode** unless noted. Hide personal folder names (use the mock).

## Stills → `docs/assets/screenshots/`
| File | Show |
|---|---|
| `chat-plan.png` | Chat, request *remove the curseforge instances that are not the 26.2 version*: plan card with versions, “Kept” card, the **Review** bar. |
| `storage.png` | Storage tab at the disk root: usage bar, folder list with bars, a couple of tips. (Grant Full Disk Access first so “Protected” is small.) |
| `history.png` | History with one entry expanded: original path, Trash location, **Put back** and **Show in Trash** buttons. |
| `models.png` | AI tab: model list with *In use* badge, and the *Add your own model* box with a Hugging Face link pasted. |
| `compact.png` | The small window pinned in a screen corner over a plain wallpaper (crop to ~360×580). |
| `light-dark.png` | Same screen (Chat with the folder cards) in light and dark, side by side. |
| `hero.png` *(optional poster)* | Chat with *what's taking the most space?* cards. |

## Animations → `docs/assets/demo/`
Record with QuickTime (⌘⇧5) or [Kap](https://getkap.co), 15–25 s each, then:
```sh
ffmpeg -i in.mov -vf "fps=15,scale=900:-1:flags=lanczos" -loop 0 out.gif && gifsicle -O3 --lossy=80 out.gif -o out.gif
```
Keep each GIF under ~5 MB (GitHub inlines them).

| File | Script |
|---|---|
| `hero.gif` | Type *remove the curseforge instances that are not the 26.2 version* → plan card → **Review** → **Approve** → “Done” → History → **Put back**. |
| `space.gif` | *what's taking the most space?* → click a folder card → it opens. |
| `instances.gif` | The version filter alone (shorter cut of hero). |
| `organize.gif` | On `Inbox`: *organize this folder by type* → approve → repeat → History → **Undo all**. |
| `rename.gif` | *replace spaces with underscores in file names* → review → approve. |
| `duplicates.gif` | *delete duplicate files* → duplicate group cards → approve. |
| `projects.gif` | *list all my projects*. |

Until the media exists the README shows broken image icons — add the files before announcing the repo.
