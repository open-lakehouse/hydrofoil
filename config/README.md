# Hydrofoil Lakehouse Configuration

## Interfaces

- [Jaeger UI](http://localhost:10000/jaeger) - Distributed tracing interface
- [Zot UI](http://localhost:10100) - OCI registry interface
- [SeadweedFS UI](http://localhost:10101) - Object storage interface

## Components

### OCI Registry

- __image__: ghcr.io/project-zot/zot:latest
- __config__: [./zot/config.json](./zot/config.json)
- __exposed_ports__:
  - `8080`: UI and API access
