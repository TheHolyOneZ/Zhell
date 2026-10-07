# Benchmarks

I wanted to know how Zhell actually stacks up against the terminals I'd otherwise use, so I
measured it. Everything ran on one machine, so your numbers will be different. What should
hold up is the order.

Setup: Arch Linux, KDE Plasma on X11, Ryzen 7 5700X, Radeon RX 6700 XT, about 2,600 fonts
installed. October 2026. Lower is better in every row.

| | Zhell | WezTerm | kitty | Konsole | xterm |
|---|---|---|---|---|---|
| Plain output, 23 MB / 3M lines | 1.02 s | 1.39 s | 0.85 s | 1.39 s | 2.42 s |
| Coloured output, 50 MB | 0.73 s | 1.23 s | 0.69 s | 1.06 s | 17.9 s |
| Launch to first output | 84 ms | 139 ms | 216 ms | 295 ms | 28 ms |
| Key press to screen, median | 12.9 ms | 30.6 ms | 17.6 ms | 18.5 ms | 12.1 ms |
| Key press to screen, p90 | 13.7 ms | 34.2 ms | 20.5 ms | 20.7 ms | 13.6 ms |
| Memory with one idle shell | 116 MB | 111 MB | 145 MB | 44 MB | 4 MB |
| Idle CPU | 0.1 % | 0.0 % | 0.1 % | 0.0 % | 0.0 % |

Typing latency and startup are where Zhell does best: it's the quickest of the GPU
terminals I tried, and only xterm (which draws on the CPU) is in the same range. For raw
output kitty is still a little faster. WezTerm and Konsole are behind on both.

Two things worth knowing when you read the table:

Zhell keeps 100,000 lines of scrollback by default, kitty 2,000 and WezTerm 3,500. When I
gave Zhell the same 1,000 lines, its plain-output time was the same as kitty's.

The memory number for Zhell includes `zhelld`, the background process that keeps your shells
running after the window closes. The others don't have one.

Running these also turned up two problems in Zhell itself. Startup used to take 164 ms
because it parsed every installed font on one core before drawing anything; it now scans them
on all cores and remembers which files your font uses, which got it to 84 ms. And the daemon
was using 0.6 % CPU while idle because it walked the whole process list every two seconds
looking for dev servers. It only looks at its own shells now.

## How each test works

Plain and coloured output: the terminal runs `cat` on a file and the shell times it. The plain
file is 3 million short numbered lines. The coloured one is 300,000 lines mixing 256-colour
and true-colour text, Unicode and emoji.

Launch to first output: the terminal is started with a command that paints the whole screen
magenta. The time is taken when that colour shows up in the window's pixels.

Key press to screen: `cat` runs in the terminal, a key press is sent through XTest, and a
small part of the window is read back until it changes. 30 presses per terminal.

Memory is the PSS of all of the terminal's processes with one shell open. Idle CPU is measured
over 10 seconds with that shell doing nothing and the window unfocused.

Every terminal keeps its own default settings. I only asked WezTerm, kitty and xterm for a
120×35 window and Zhell for 1000×640 pixels, which comes out about the same. Output tests ran
3 times, startup 5 times; the table shows the median.

Latency is measured to the window as X sees it. The compositor and the monitor add their own
delay on top, but that's the same for every terminal.

## Running them yourself

You need an X11 desktop, Python 3 and the terminals you want to compare.

```sh
cargo build --release
python3 bench/gen.py                              # writes the test files to bench/out/
python3 bench/run.py zhell,wezterm,kitty plain    # also: color, idle
python3 bench/visible.py zhell,wezterm,kitty      # launch to first output
python3 bench/latency.py zhell,wezterm,kitty      # key press to screen
```

Zhell gets its own daemon, settings and state under `bench/out/` for this, so it won't touch
your real sessions. Close anything heavy first. A compile running in the background will
throw off every number.

## Startup in detail

`RUST_LOG=zhell=debug,zhell_render=debug zhell` prints how long each step takes. On the test
machine the window exists after about 11 ms and the GPU is ready at around 25 ms. Font work for
the first frame is close to zero, because Zhell remembers which files belong to your font in
`~/.cache/zhell/font-quickstart`. The full font list, needed for emoji, CJK and other fallback
characters, is loaded in the background in about 45 ms and swapped in once it's ready.
