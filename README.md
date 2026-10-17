[(try it here)](https://sal-ami.github.io/lebronify/)

# lebronify

a new technology that turns any image into LeBron James. nobody asked for it, and that has never stopped a project like this before.

You hand it a picture and it hands the same picture back with the pieces rearranged until the whole thing looks like LeBron. Nothing gets added and nothing gets thrown away, because the program only ever moves pixels from one square to another.

## How to use

**Drop an image into the source box, change the settings if you are the type to change settings, and press start.** The grid rearranges itself live while the solver runs, and when it finishes you can replay the change as an animation or save the result as a gif. the lover the proximity and the higher the grid size(resolution), the higher the quality and the time it takes to finish. *NOTE: i would recommend that proximity should be at "6" and grid size(resolution) be at "168"

| Setting | Description |
|---|---|
| resolution | how many cells the picture is cut into. More cells keeps more detail and takes longer to solve. |
| proximity importance | how far a cell may travel from where it started. Turn it up for a subtle rearrangement, turn it down and the solver stops respecting the original image. |
| algorithm | fast is the default and finds a good arrangement in seconds. optimal finds the best possible one and will make you wait a long time for a difference you almost certainly cannot see. |

### What you can upload

The site takes images and video.

| input | file types | result |
|---|---|---|
| image | png jpg jpeg gif bmp webp tif tiff | the image rebuilt out of LeBron |
| video | mp4 webm mov | an animated gif of the same idea |

Images of any common format work. An animated gif is treated as a single frame, and you get a single frame back.

Video is the slow one. The site samples frames across the clip and solves each one toward LeBron before stitching them into an animated gif. A few seconds of footage can take a while, and your laptop will start making a noise it does not normally make.

#### The command line tool

A separate command line tool handles more than pictures. It reduces each medium to the same square grid and moves the cells around. Run `lebronify tiers` to see the list straight from the program.

| medium | file types | what the grid shows |
|---|---|---|
| image | png jpg jpeg gif bmp webp tif tiff | the picture |
| document | txt md | the page as ink |
| website | html htm | the layout as boxes |
| audio | wav mp3 ogg flac | a spectrogram |
| waveform | mid midi | a piano roll |
| mesh | obj stl ply | a shaded 3D model |
| table | csv tsv json | a heatmap of the numbers |
| source code | rs py js ts go c cpp h | code colored by token type |
| palette | hex gpl ase | the swatches as a ramp |
| vector | svg | not built yet |
| video | mp4 webm mov | not built yet, the site already does video |

## Installations

The site is already running at the link at the top, so if you came for the website you can stop reading here and go ruin an afternoon.

Building the command line tool from source:

1. Install [Rust](https://www.rust-lang.org/tools/install)
2. Run `cargo run --release -p lab-cli -- tiers` in the project folder

Running the web version locally:

1. Install [Rust](https://www.rust-lang.org/tools/install)
2. Add the wasm target with `rustup target add wasm32-unknown-unknown`
3. Build the engine with `cargo build -p lab-web --target wasm32-unknown-unknown --release`
4. Start the server with `python tools/serve.py` and open http://localhost:8000

The server only hands the page its engine file. It never sees your data.

## Contributing

I really don't care but I will try to maintain.

## How it works

The engine shrinks your image into a square grid where every cell is one color. The solver then searches for the arrangement of those cells that matches the LeBron target best, scoring each arrangement by how far the colors land from the target and by how far each cell had to move to get there. Lower scores are better, and the answer it returns is a list of where every cell went. The picture is only one way of looking at that list.

None of this is original. It is built on top of [obamify](https://github.com/Spu7Nix/obamify) by Spu7Nix, which does the same thing with Obama and does it better. The cost model, the solver, and the target image are all taken from there, so go and look at the original.

## License

MIT. See LICENSE.

the extension code: https://github.com/Sal-ami/lebronify-extension
