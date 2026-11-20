# skp-open

Open SketchUp (`.skp`) conversion, in both directions, with no SketchUp and no Trimble SDK.

```sh
skp-open convert model.rvt model.skp                    # the 2026 container (SketchUp 2026 and 2027; 2021 to 2025 open it)
skp-open convert model.rvt model.skp --release 2017     # the stream container, release 2017 (any year 2013 to 2020, or 3 to 8)
skp-open convert model.skp model.ifc                    # .ifc .ifcx .glb .obj .usda .usdc .frag .dxf
skp-open info model.skp                                 # counts; for a .rvt, its identity
```

- `.skp` is read and written by [skpwright](https://github.com/hew3d/skpwright) (GPL-3.0-only).
- `.rvt` is read, and IFC4, IFCX, glTF, OBJ, USD and Fragments are written, by [rvt-open](https://github.com/helenkwok/rvt-open) (Apache-2.0), pinned to a commit in `Cargo.toml`.
- skp-open links skpwright, so it is GPL-3.0-only. rvt-open never depends on it.

## What maps to what

SKP to the other formats (`src/from_skp.rs`): each group or component instance is one building element holding the triangles of its own faces in world coordinates, one material per triangle. An instance that places others is an element assembly that aggregates them. Names, layer, definition and attribute dictionaries are property sets. Hidden instances, faces and layers are left out. Textured materials keep their average colour. The DXF is 3D, one `3DFACE` per triangle on the SketchUp layer, in metres.

RVT to SKP (`src/to_skp.rs`): the graph rvt-open writes IFCX from (project, site, building, storeys, elements, bodies) becomes groups. Every node holding a body or a child is a group; the IFC class is the layer; a body's colour is a solid material; the node's properties are an attribute dictionary named `rvt-open` on the definition. Triangles become faces with welded vertices; the edge between two flat neighbours of one material is hidden, as a quad's diagonal is.

## Checks

`cargo test` runs a model built in code through every writer and back. With `SKP_OPEN_FIXTURES` set (`tools/ci/fetch-fixtures.sh` downloads public inputs against pinned hashes) it also converts skpwright's corpus and rvt-open's public models, and holds the counts and bounding boxes against skpwright's own reading.

