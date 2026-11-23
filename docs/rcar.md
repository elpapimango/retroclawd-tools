# rcar: object libraries

```
rcar create lib.rca a.rco b.rco [--force]   make a library
rcar list lib.rca [--symbols]                members, their sizes; with --symbols what each exports and imports
rcar extract lib.rca [member...] [-d DIR]    write members out (all when none named)
rcar add lib.rca x.rco                       add objects, replacing members of the same name (creates the library)
rcar delete lib.rca a.rco                    remove members
```

A library is a list of `.rco` objects and an index of the names they export (global labels and constants). `rcld` reads the
index: it links a member only when something still needs a name that member defines. The members of a library may import from each
other. Member names are file names; a file that is not an object is an error naming it. The format is in `docs/formats.md`.
