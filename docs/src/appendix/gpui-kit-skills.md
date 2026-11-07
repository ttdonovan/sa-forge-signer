---
description: "Optional: install GPUI Kit's Claude skills (gpui-kit, gpui-kit-design-guides) at the console's gpui-kit version with just apps gpui-kit-skills; where they go, updating and removing them"
---

# GPUI Kit skills

The [console](../apps/console.md) is built with [GPUI Kit](https://gpui-kit.com), whose authors publish two Claude skills: `gpui-kit` (the Coding Guides, component conventions and GPUI mechanics) and `gpui-kit-design-guides` (the Design Guides). [Building an app](../dev/apps.md) says when to read them; installing them lets Claude Code load them itself.

```sh
just apps gpui-kit-skills               # into ~/.claude/skills
just apps gpui-kit-skills --project     # into this repository's .claude/skills (ignored by git)
just apps gpui-kit-skills --uninstall   # remove what it installed
```

`scripts/skills/gpui-kit.sh` reads the `gpui-kit` version from `apps/console/Cargo.lock` and fetches only `skills/` from <https://github.com/longbridge/gpui-kit> at the matching tag (`--tag vX.Y.Z` picks another), so the guides match the version the console builds against. Run it again after upgrading `gpui-kit`. It needs only `git`.

Each skill folder it installs gets a `.installed-by-sa-forge-signer` file naming the tag and commit. It replaces only folders carrying that file and refuses any other skill folder of the same name; `--uninstall` removes only marked folders.

The skills are third-party instructions: read each `SKILL.md` before installing. Installed in `~/.claude/skills`, they load in every Claude Code session on the machine, not only in this repository; `--project` keeps them to this checkout. Claude Code picks them up in its next session.
