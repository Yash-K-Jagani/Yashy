# Shop

A complete persistence example: HTTP + SQLite + checksummed migrations.

```console
$ yashyc run main.ya
shop-ok
```

With a port argument the same program serves the CRUD operations:

```console
$ yashyc run main.ya listen 8443
```

`load.ya` is a single load worker (`load <port> <count>`); it prints `ok/total`.
The 50-client load row is a dated exception in `docs/gates.md`.
