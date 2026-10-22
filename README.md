![Project Header](./screenshot.png)

# RA2 Engine

A rewrite of the original RA2 engine written in Rust with [Bevy](https://github.com/bevyengine/bevy), that plays
Command & Conquer: Red Alert 2 (in a limited way, see [known issues section](#known-issues)) from your own copy of
the game.

> [!IMPORTANT]
> Red Alert 2 is a trademark of Electronic Arts. This project is not affiliated
with or endorsed by Electronic Arts.
>
> **No game files are included.** You need your own copy of the original Red Alert 2 game files
(for example, by purchasing on Steam).

## Playing 
The easiest way is to download the latest release here for your operating system (currently only macOS) and open the 
app. It'll prompt you for the directory where your game files are and once selected will start the game. 

There is a helper utility to force download the game from steam on mac, since the steam app doesn't allow downloading in 
the UI since mac isn't a supported OS. It will open the console tab on the steam app and give you a command to paste 
that will download the game files if you've purchased it. The app will then monitor the download for completion and pull
in the files when they're done.

Currently only skirmish games are supported.

## Building from Source
Eventually this repo will share releases for all major targets as well as hosting a web version. In the meantime (or if 
you want to modify the source/run the tests) you'll need to copy your game data into the repo directory within a 
directory called `app_2229850` (the directory name as downloaded from steam) to be able to run it directly.

If you don't have a rust toolchain installed, you'll need to [do that first](https://rustup.rs/). Afterwards, you can 
run the game from source with:
```shell
cargo run
```

Or to run the optimised release version:
```shell
cargo run --release
```

To run the tests:
```shell
cargo test --workspace --release
```
Tests that need the game's files (in `app_2229850`, or the folder `RA2_DIR` points to) are marked ignored when they
aren't there, so the rest still run.

## Known Issues
> [!NOTE]
> This is a mostly vibe-coded rewrite and is not a direct translation of the original game. The focus of this project
is to get the game running as close to the original as possible with as much portability as possible. As such, there 
have been numerous bugs and inconsistencies I've noticed during the creation of this project and there'll likely be a 
lot more I haven't noticed.
>
> If you find issues during a game, you can save the game and then create a github issue describing the issue along with
your save file. There is tooling within this repo to step through a saved game in order to more easily debug these types
of issues without guessing what happened.

- No campaign support
- Yuri prime's have no creation limit, and their range seems incorrect
- Missing animations for various entities (as well as some incorrect ones)
- Team allying not supported
- No hosted online server

## Project Structure

| Crate | What |
|---|---|
| `ra2-formats` | Readers for the game's file formats (MIX, SHP, VXL, HVA, PAL, CSF, INI, maps). |
| `ra2-sim` | The simulation: deterministic, no graphics. |
| `ra2-net` | Lockstep network play. |
| `ra2-server` | A small relay and lobby server for internet games. |
| `ra2-tools` | Command-line tools for looking inside the game's archives. |
| `ra2-bundle` | Builds the Mac app and disk image. |
| `src/` | The game: drawing, sound, menus and input. |

## Motivation & Alternatives
There were a few different motivations for this project which, depending on what you're looking for, might mean your 
expectations of this project fall short. If that's the case for you, one of the alternatives listed is probably a much 
better fit.

This is mostly a personal project to explore 2 different areas where I have little exposure:
- Clean room reverse engineering
- Game development (an excuse to try out Bevy)

Red Alert 2 was probably the first game I played as a kid and over the years I've purchased the game multiple times 
(old discs off of eBay as well as the steam version) to play a few LAN games with my dad and brother every now and then. 
Each time I've done this, there's been a lot of issues from graphical issues trying to run it on Windows 7 to 
downloading patches for the networking, right through to spinning up Windows XP in virtualbox to get it running on a 
Mac. Since we never really played Yuri's Revenge or the other C&C games, the goal of this project is purely RA2 
focussed. 

With that in mind, the game goals of this project for me were for it to be:
- Portable: I wanted to be able to run this as a native Mac app, as well as in the browser.
- Realistic: I want it to feel like the original game (I personally am not interested in modding), e.g. unit movement 
should feel original, music, sound effects and animations are important 
- Simple: I want the setup to be as frictionless as possible, with a simple networking implementation out of the box for
both LAN and the Internet.

If I exclude my personal goals with this project, it would've been more efficient to go down the OpenRA route with the 
RA2 mod and then modify it to get it running with less friction for RA2 like it does for the original RA, as well as 
building for the browser. 

When should you not use this project?
- You want to mod this a lot without re-compiling. OpenRA's lua-based scripting is a much better approach for this kind 
of use case and a fair amount of work would likely need to be done to get this to support something similar.
- You want to play other C&C games, including Yuri's Revenge. OpenRA is what you want here, this is their intended goal
of the project. This project may expand to include YR in the future, but right now it's at the bottom of the feature 
pile for me personally since I very rarely played it. 
- You want competitive online play. The ethos for the current online implementation is much closer to the original's in 
terms of how it works, which has some real drawbacks if you were playing this competitively, e.g. you could easily 
modify the client to just show the entire map since all player actions are distributed to everyone and the shroud is 
just a client-side render. The idea behind this online play functionality is just a casual with no accounts/sign up, etc.

### Alternatives
Both alternatives are much more full-featured than this and if you want modding or better online features go with those.
On the other hand, if you're looking for a purely RA2 focused open source engine then this might be useful.  

- [OpenRA](https://www.openra.net/): Flexible and customisable game engine for a variety of C&C games, with a mod to support RA2. Written in C# 
and afaict follows an EC game model.
- [Chrono Divide](https://chronodivide.com/): Browser based RA2 focused on online play and a demo mode that is similar to a skirmish from the 
original. Engine is not open source afaict.

## Roadmap
Planning to add the following functionality roughly in this order:
1. Browser support with a hosted option, similar to Chrono Divide (see [alternatives](#alternatives)) but with a focus 
on more flexible skirmish setup instead of the demo mode, and with a closer to the original implementation in terms of 
bot setup and behaviour.
2. Hosted server to allow for online multiplayer with no login/account setup.
3. Campaign support.
4. Other native OS releases.
5. Optional additional bot behaviour modes that are more human-like, e.g. react to known information, don't have 
starting gold advantages, etc.
6. iOS port with custom controls.

## Issues
Feel free to create issues directly in Github, and optionally upload game saves for easier debugging of those issues.

## Contributing
Feel free to open a PR, although I make no guarantees about getting it merged if it doesn't fit with the spirit of what
the project aims to achieve. Alternatively you can fork it and make it your own, or just depend on some of the internal
packages instead.
