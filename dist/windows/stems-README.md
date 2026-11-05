# Stem Splitter files for the installer

`dist/windows/stems/` is not in git. Before building the installer it must hold:

- `demucs.exe`: from https://github.com/nikhilunni/demucs-rs at commit `805d6f72ede20282832ac78c634e8767716c0b21`, built with `cargo build --release -p demucs-cli`. Apache-2.0.
- `LICENSE`: that repository's licence file.
- `htdemucs.safetensors`: the 84 MB model, from https://huggingface.co/set-soft/audio_separation/resolve/main/Demucs/htdemucs.safetensors

**Model licence:** unresolved. The Demucs code is MIT, but no explicit licence for the trained weights was found, and reports quote a project statement that they were for research use. The owner chose to bundle them while Pro Wave is not sold. Revisit this before any sale, or replace the model with one of our own.

The helper needs a working Vulkan GPU driver. It is also built for CPU with `--features cpu`.

For a development build, set `PROWAVE_STEMS_DIR` to a folder holding these files.
