# Robo Sutra splash artwork

The welcome screen uses the candidate title **Robo Sutra**. Continue opens the
dashboard; Help → Welcome to Sutra reopens the introduction. The preference is
`hasSeenIntroduction`. The app bundle and project remain Sutra.

This is native SwiftUI Canvas / Core Graphics artwork, intended for the splash
screen and large App Store listing graphics. A 30 fps timeline drives the scene.
No raster sprites, SVG loader, custom Metal shader, or external package is needed.
Reduce Motion shows the quiet pose. Occlusion pauses the clock and resumes the
same performance when the window becomes visible.

## Choreography

The three robots surround an open patch of the perspective board. The rear robot
is smaller; the two foreground robots frame the action. The mat's sides and front
extend past the viewport. Four central ports—3000, 3001, 6006, and 5173—stay free
for calls. Every four calls use all four destinations in a freshly shuffled order,
with no consecutive repeated port even across shuffle boundaries. The two yellow
targets support hand and foot races; the front green targets support foot races
won by either foreground robot. All three still race toward the called circle.
There are sixteen valid port/limb/winner combinations; six visits to each port
cycle through its shuffled contenders. Each launch chooses a new seed. Calls omit
robot names and colors.

Each performance lasts six seconds:

- The call appears; after a brief weight shift, the first robot launches at 0.55 seconds.
  Speed peaks early: the reaching limb covers 37% of the distance in its
  first quarter, then brakes smoothly into its landing.
- The winner plants at 1.40 seconds, compresses briefly, then holds its new pose.
- The first loser arrives at 1.85 seconds, overreaches, squints, and pulls back.
- The second arrives at 2.22 seconds, hesitates with a wide-eyed reaction, then retreats.
- The call fades; the winner visibly returns to its starting dot from 4.65–5.75 seconds.

All three named limbs travel toward the same circle. Losing limbs stop 22 points
short while airborne; they never share the winning dot. Other limbs stay planted
on their twelve individually reserved supports. The end of each performance
returns continuously to the starting arrangement, making the next call legal.
This is a directed illustration loop, not a persistent game simulation.

Bodies lean and translate with the reach and stay almost still while planted.
Winning bodies stay committed over their new contact through the hold. Two-link
IK solves fixed-length bones in depth; their drawn sections foreshorten instead
of folding flat against one another. Foreground hands rest on the outer yellow
dots, with feet on the corresponding outer green dots. This gives them a wider,
uncrossed starting stance and keeps the middle open. Heads are more upright at
rest. Joint swivels favor open silhouettes and blend smoothly as the body leans,
including during the quicker launch. Foot lifts stay low. Subtle body
scaling and head foreshortening accompany travel. The target has a floor ring and
a brief landing ripple. There are no dotted guides, checkmark badges, or X icons.

## Drawing and validation

Heads, torsos, joints, hands, feet, and limb sections share a depth-sorted draw list.
Long limb segments use clipped depth slices so opposite ends can pass behind and
in front of another robot. At crossings, the ordering is held across the entire
width of the overlapping limbs. Shadows remain on the floor. Winning does not
change an entire robot's layer priority.

The checks exercise 9,600 performances across 100 seeds: all four targets in each shuffle, no
consecutive repeated ports, unique supports, free calls, fixed segment lengths, three directed attempts, exactly one landing,
staggered near misses, visible targets, continuous returns, and quiet idle motion.
Timing checks verify an early burst of speed and smooth takeoff/landing endpoints.
A denser 60 fps pass checks elbows and knees for snapping; joint-angle checks
reject deep folds, and every joint must stay above the board. Pixel checks cover
limb/body occlusion in both directions and submission orders, slice seams, and
limb/limb crossings. These checks support, but do not replace, visual inspection.

## Preview and exports

From the repository root:

```sh
macos/preview-intro.sh
macos/preview-intro.sh --gif /tmp/robo-sutra.gif --duration 6 --seed 33
macos/preview-intro.sh --snapshot /tmp/hero.png --time 1.55 --seed 33 --cue
macos/preview-intro.sh --snapshot /tmp/hero-transparent.png --time 1.55 --seed 33 --transparent
macos/preview-intro.sh --check
```

The preview runs without the Rust core, project scanning, or changing Sutra's
preferences. PNGs are 2048 square; `--cue` includes the call and target ring. GIFs
are 640 square at 20 fps; the default duration is 24 seconds. A six-second export
shows one complete performance. The live app runs at 30 fps and continues through
the shuffled repertoire. Fixed seed and time values reproduce a chosen frame at
any resolution. `PortDance(time:seed:)` exposes the same option to SwiftUI.

The check command writes a six-phase contact sheet for the first four calls to
`macos/.build/intro-preview/poses.png`. To view 24 calls at a chosen phase:

```sh
macos/preview-intro.sh --check /tmp/held-poses.png 3.6
```

Build the full app with `macos/build.sh`, or, when the Rust bindings and XCFramework
are already in `macos/build`, `swift build --package-path macos --build-system native`.
Existing logo/icon assets are unchanged.
