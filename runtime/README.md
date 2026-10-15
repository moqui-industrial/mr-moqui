# runtime

The runtime directory holds everything that varies per instance. The binary finds it through `MOQUI_RUNTIME`
(default `runtime`).

| Path | In git | Purpose |
|---|---|---|
| `base-component/` | yes | The framework and base components that ship with Moqui |
| `component/` | no | Additional components, normally checked out by `./moq` |
| `conf/` | yes | Runtime conf files (`MoquiDevConf.xml`, `MoquiProductionConf.xml`, ...) merged over `MoquiDefaultConf.xml` |
| `cert/` | no | HTTPS certificates by domain: `<domain>.crt` + `<domain>.key` (nginx-proxy), `<domain>/fullchain.pem` + `privkey.pem` (certbot), or `<domain>.pem` + `<domain>-key.pem` (mkcert). `_wildcard.<parent>`, `<parent>` and `default` are fallbacks. Dev generates a `localhost` certificate when empty. Never commit keys. |
| `db/` | no | Embedded database cluster and related files |
| `log/` | no | Log files |
| `opensearch/` | no | Managed OpenSearch install and data |
| `tmp/` | no | Scratch files |
| `txlog/` | no | Transaction logs |
| `.env` | no | Generated secrets and local overrides |

Directories marked "no" are listed in `.gitignore` and are created when first needed.
