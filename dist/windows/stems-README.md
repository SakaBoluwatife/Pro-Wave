# Stem Splitter files for the installer

`dist/windows/stems/` is not in git. Before building the installer it must hold:

- `demucs.exe`: from https://github.com/nikhilunni/demucs-rs at commit `805d6f72ede20282832ac78c634e8767716c0b21`, built for the CPU with `cargo build --release -p demucs-cli --features cpu`. Apache-2.0.
- `LICENSE`: that repository's licence file.
- `htdemucs.safetensors`: the 84 MB model, from https://huggingface.co/set-soft/audio_separation/resolve/main/Demucs/htdemucs.safetensors

**Model licence:** unresolved. The Demucs code is MIT, but no explicit licence for the trained weights was found, and reports quote a project statement that they were for research use. The owner chose to bundle them while Ovrtone is not sold. Revisit this before any sale, or replace the model with one of our own.

The owner chose the CPU build for now, to optimize later. The GPU build crashed on the owner's laptop (Intel UHD 620 and NVIDIA MX150) with a wgpu "Encoder is invalid" error on a 10-second test tone.

For a development build, set `OVRTONE_STEMS_DIR` to a folder holding these files.
