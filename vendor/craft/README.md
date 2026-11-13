# WordCraft, GridCraft and DeckCraft tools, our build (an add-on)

The viewer shows office documents as PDF, converted by the Craft tools'
command-line programs: `wordcraft-cli` (docx, doc, odt, rtf), `gridcraft-cli`
(xlsx, xls, ods) and `deckcraft-cli` (pptx). The released macOS CLIs are
universal and embed the craft-fonts pack (Japanese, Chinese, Korean and other
faces); this build is for one processor, without the pack (documents use the
system's fonts), fully optimised and stripped.

It is not in git: `vendor/craft/build.sh` builds them from their source
(MIT or Apache-2.0; `<name>.txt` beside each build holds the MIT licence and
NOTICE) into `<os>-<arch>/`, for the machine it runs on (Windows: in Git
Bash; for ARM64, Git Bash started from ARM64's developer prompt, then
`PLATFORM=windows-aarch64 vendor/craft/build.sh`, set in bash: the prompt's
own `Platform` hides a `set PLATFORM`; Git Bash's `link` hides MSVC's, so
`CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER` names its `Hostx64\arm64\link.exe`).
The app uses only
these, or the add-on; never a tool installed on the system.

They are an add-on, apart from the app: `packaging/addons.sh` zips them with
their licences, signed as the app is on macOS, and Settings > Add-ons
downloads them.
