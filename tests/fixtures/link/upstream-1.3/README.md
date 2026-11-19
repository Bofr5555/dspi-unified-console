# DSPi Link 1.3 fixtures (WP PGMAT)

Copied unchanged from **WeebLabs/DSPi-Link** at commit
`4aec5ab293be4d7f7c515bf28f7d5aaec2e14b1f` (2026-11-17, "Harden the
legacy-firmware guard after an independent audit"; the fixtures last changed in
`0fa5250`, "Attach firmware 1.1.7 and port the clients to command IDs"): spec
1.3, firmware 1.1.7. Taken with `git show 4aec5ab:fixtures/<path>` from a local
clone (read only), 2026-11-19. MIT licence (`LICENSE.DSPi-Link`, upstream's
`LICENSE` unchanged, Copyright (c) 2026 Weeb Labs; the same text as
`../LICENSE.DSPi-Link`). The JSON and binary files carry no header of their own.

The 1.2 fixtures beside this directory (`../frames`, `../messages`, DSPi-Link
`86f10e3`) stay as they are; this directory holds the files 1.3 changed
(`frames.json`, the two CMD request frames, `hello_*`, `auth_ok`,
`device_list_ok`, `poll_subscribe`) and the two requests the replay needs
unchanged from 1.2 (`device_list.json`, `poll_subscribe_ok.json`).

| File | Upstream path | Used by |
|---|---|---|
| `frames/*` | `fixtures/frames/` | `crates/dspi-hub/tests/wp_pgmat_hub.rs`: the L13 client's CMD frames (`ROUTE.MASTER_VOLUME` SET, `SYS.PLATFORM` GET), replayed with the handle byte set to the attached device's |
| `messages/*` | `fixtures/messages/` | the same test: the L13 client's `hello` (minor 3), `device.list`, the device object's shape, `poll.subscribe` |

SHA-256 (`sha256sum`):

    e560cd42a817fa8d9e1aab94ca40abecc941038e365c5ac5dbebb3568b12f07e  ./frames/cmd_request_get_platform.bin
    86ae1a312661aa1b669a19c6f5cf350179171982f538d8b6fb1696a6ee975d35  ./frames/cmd_request_set_master_volume.bin
    41d81c46b6795a57b26851f2b987f3bb46c952128aa11d6018527c519587f8f9  ./frames/frames.json
    d3d8951f2393213f59fa4033f6637bf0b9cd20845f10e4bbcf3923b60a937e51  ./LICENSE.DSPi-Link
    531bfb093c0905844832f82a97f316598bb85464f2b007a0b5f532f88a89c925  ./messages/auth_ok.json
    30c083c0ef09b252d8c79d879d51e61d043297f1c7a13c93e4da5d144ffc6e54  ./messages/device_list.json
    349ee79d9516f0fb110d966c15b14c23a5c6f37f9c16921d5f9446299d8c4af6  ./messages/device_list_ok.json
    65b8d07ccab93428666b4a0534053e7b15353816add1805c5d0aed68cc5ee216  ./messages/hello_client.json
    1d6498c72c069ba56a95a800c6e58c8c8c638967750b226362fe7c20e3bb4856  ./messages/hello_hub_bridge.json
    3a2ba8fcbf1ccfd3dcaa40eb7d36dcc595376ab7438ffc5407fb67b86cbbb4d1  ./messages/hello_hub.json
    c686ec9fce537e4834fa728d13bc81ecd64439a662ce55f6455d9d10609bb335  ./messages/poll_subscribe.json
    39d09aa20f3d0a72cccefc91e1c7594a43ed7ca95fb937e0391fe25f6a767c23  ./messages/poll_subscribe_ok.json
