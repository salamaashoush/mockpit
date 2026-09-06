# Changelog

All notable changes to this project will be documented in this file.

## [0.4.1] - 2026-09-06

### Bug Fixes

- *(create)* --template wrote a function that does not exist ([2971852](https://github.com/salamaashoush/ferrimock/commit/2971852fd194134682d10347e75b0d5db901df8f))

### Features

- *(cli)* Expose the mock and fake operations without clap ([9801a01](https://github.com/salamaashoush/ferrimock/commit/9801a01c66cfe5b52a64339795d0d117970acd55))

## [0.4.0] - 2026-08-25

Six breaking changes, all in the state machines and the world doctor added
since 0.3.0. Nothing in the mock engine, the templates or the recorder moved.
A `MockConfig` or `MockCollectionConfig` built field by field will not compile,
and neither will a `match` over `doctor::Check`. See
[UPGRADING.md](UPGRADING.md).

The headline feature is `ferrimock proxy`: the engine in front of a dev server
or a real backend, on one origin, answering from mocks and forwarding
everything else.

### Bug Fixes

- *(doctor)* Measure vocabulary where it means something, and how ([8728943](https://github.com/salamaashoush/ferrimock/commit/8728943379e7a3492db2f2e148205d70a86478a9))
- *(doctor)* A key is not a field that is never absent ([06631da](https://github.com/salamaashoush/ferrimock/commit/06631dad1fe8874cc4b7678cd8bbdda4066ad61b))
- *(doctor)* An id tracks creation, not every timestamp beside it ([de35969](https://github.com/salamaashoush/ferrimock/commit/de359694c42f1421355b0ed8d64ccc2308b21b04))
- *(doctor)* An enum needs enough draws before flat means flat ([61e9c2a](https://github.com/salamaashoush/ferrimock/commit/61e9c2ac26f605fafb501f15b76dcee49f068f5f))
- *(doctor)* A key numbers the census, it does not draw from a bound ([f919f20](https://github.com/salamaashoush/ferrimock/commit/f919f207a0897ad6149780276e3b939045e15745))
- *(doctor)* A date repeated is not a date drawn again ([251a388](https://github.com/salamaashoush/ferrimock/commit/251a388066a42bdfebd4c7ea13381446b8274e7a))
- *(doctor)* A floor set at the average skew cannot see half of what it draws ([51776c9](https://github.com/salamaashoush/ferrimock/commit/51776c9497992ee76c4331ca64bd98acbcca398e))
- *(graphql)* A list of ids is not the field that addresses one record ([f370683](https://github.com/salamaashoush/ferrimock/commit/f37068319564eed8dcbc505fef2f8a034617f357))
- *(fake-data)* A flag a template draws is not a fair coin either ([b304e89](https://github.com/salamaashoush/ferrimock/commit/b304e894fa0c147542dfa4a92df07e3bb2acd5d8))
- *(config)* A machine an editor cannot complete is a machine nobody writes ([ee9c5b1](https://github.com/salamaashoush/ferrimock/commit/ee9c5b11a90dd6e13ac851165d43d09a8050cfa9))
- *(cli)* A clap help string is rustdoc as well ([1cd464d](https://github.com/salamaashoush/ferrimock/commit/1cd464df9dfae1b1a5fffaaae20bb26a56ae89c9))
- Give the trait solver the depth the mock loader's future needs ([95a6e15](https://github.com/salamaashoush/ferrimock/commit/95a6e157c6413a1c57c34e05a59c8d434e197da1))

### Documentation

- *(doctor)* A public doc cannot link to a private constant ([855a70d](https://github.com/salamaashoush/ferrimock/commit/855a70d403db700b8df2ae24959ec926cdd4382d))
- *(proxy)* What it costs, measured against a direct baseline ([1540490](https://github.com/salamaashoush/ferrimock/commit/154049052c941dd79568d9477622630c4a440eed))

### Features

- *(fake-data)* A vocabulary deep enough to write with ([23d39f3](https://github.com/salamaashoush/ferrimock/commit/23d39f34f2acb83f8550f34d17c784cdc7b9a96c))
- *(doctor)* A lint reads the world it needs, not the one that is served ([7270a31](https://github.com/salamaashoush/ferrimock/commit/7270a31eb26d2f2f454634e07fe0ee209bb7a999))
- *(machine)* A machine is declared once and named, and its edges are the graph ([2ed9a5a](https://github.com/salamaashoush/ferrimock/commit/2ed9a5a86a782987a4aaeeb0b4316a20cf1883a6))
- *(machine)* An instance exists when something asks for it, and its edges are counted ([873fa3e](https://github.com/salamaashoush/ferrimock/commit/873fa3e80d78d759128bd67f26034d2fdcd95d24))
- *(machine)* Machines drive routes, with no schema anywhere ([199a892](https://github.com/salamaashoush/ferrimock/commit/199a89238dea4a4bfb6ad4710343a2114f9ccfde))
- *(machine)* A move can come from time, and an edge can come from anywhere ([868fbe7](https://github.com/salamaashoush/ferrimock/commit/868fbe74bffdbb3a015547502cc1a90179877865))
- *(doctor)* A branch nothing took is a coverage question, and a state nothing reaches is a defect ([348398c](https://github.com/salamaashoush/ferrimock/commit/348398cbbdf2724720939aac35be67ab3dbea54c))
- *(machine)* When, states and fire, lowered into the template they mean ([766b5f0](https://github.com/salamaashoush/ferrimock/commit/766b5f0b467e46149955580a898012e6cae7fde1))
- *(doctor)* The spec is an oracle, and consulting it found three real defects ([0a68ed4](https://github.com/salamaashoush/ferrimock/commit/0a68ed4a4126b4e046006f9c6766a7556707d65a))
- *(doctor)* Name the arrival pile, and give strict a way to be adopted ([6975a41](https://github.com/salamaashoush/ferrimock/commit/6975a412799b4be2ea8494a122529de256b2ae27))
- *(proxy)* A mock answers, or the real thing does, on one origin ([8a116ec](https://github.com/salamaashoush/ferrimock/commit/8a116ec71c1e0ce4271bd65279669ecb3b6a2f93))
- *(cli)* Ferrimock proxy, pointed at a dev server or a backend ([c4f6842](https://github.com/salamaashoush/ferrimock/commit/c4f6842511cc56c44cc0eae9bf2e3d6064372bb4))

### Refactoring

- *(machine)* States and the moves between them are not an entity's idea ([1d2e783](https://github.com/salamaashoush/ferrimock/commit/1d2e7833431d2e62199c41955491be13aad597b2))

### Testing

- The six ways this codebase failed quietly, each given something that watches ([0cf7cf0](https://github.com/salamaashoush/ferrimock/commit/0cf7cf0cb1f1018b99284ec276d543cabcf2ad7c))

## [0.3.0] - 2026-08-21

Nine breaking changes, all in the entity world and the spec-derived
backends. A world built from the same seed is not the world it was in
0.2.0 — different instance counts, different ids, different timestamps,
and optional fields that are now genuinely sometimes absent. See
[UPGRADING.md](UPGRADING.md) before upgrading.

### Bug Fixes

- *(lint)* Satisfy the clippy lints a newer nightly enforces ([7620c84](https://github.com/salamaashoush/ferrimock/commit/7620c84fee3086c785252b72cf7ad112f58780fc))
- *(template)* Keep structured responses through leading whitespace ([a8eaff7](https://github.com/salamaashoush/ferrimock/commit/a8eaff76733b5082710739b85424632d02411959))
- *(napi)* Surface the message a throwing handler actually threw ([1283220](https://github.com/salamaashoush/ferrimock/commit/1283220dd2ca9ccbb5382907988b3a7b5c4bc6c2))
- *(har)* Make a converted recording replay itself ([57aa883](https://github.com/salamaashoush/ferrimock/commit/57aa883e1ed71bc140a5620eda4c7dc6c2ba9e16))
- *(har)* Index the sequencing groups without panicking ([c353f81](https://github.com/salamaashoush/ferrimock/commit/c353f81462a6eba415da8564b3197a565d646145))
- *(lint)* Satisfy the checks CI runs but a local clippy does not ([9173549](https://github.com/salamaashoush/ferrimock/commit/9173549f93f344afa950afe791719098e95195dc))
- *(consolidation)* Answer with the id the URL asked for ([3ad593f](https://github.com/salamaashoush/ferrimock/commit/3ad593f086ad3d0048add0d4a80d64277cfe26b1))
- *(type-detector)* Keep an id the JSON kind it was recorded as ([b32c262](https://github.com/salamaashoush/ferrimock/commit/b32c26253f3b4915d57b601aca1b2a42c413e67e))
- *(consolidation)* Stop a partition outranking the template it split from ([c578662](https://github.com/salamaashoush/ferrimock/commit/c5786620997e02728fd2c87a95e105e4ea137946))
- *(codegen)* Answer a mixed listing with the kinds it recorded ([e4f7658](https://github.com/salamaashoush/ferrimock/commit/e4f7658b18ad2a89239acb27ff6ed371d5e57755))
- *(har)* Read a recording whose version the parser refuses to name ([7a8fccb](https://github.com/salamaashoush/ferrimock/commit/7a8fccbe321af29dd274e7a3d14d0e97f3dbfaaa))
- *(type-detector)* Read a value for what it is, and keep how it was written ([98802d0](https://github.com/salamaashoush/ferrimock/commit/98802d0bacb145fe9ea1abe7588e4d7c9ac454a8))
- *(type-detector)* Read a flag spelled both ways, and keep the spelling ([6d9a033](https://github.com/salamaashoush/ferrimock/commit/6d9a0333a8f069528dde17f4c46134975a0e2ce8))
- *(graphql)* Keep every type wrapper, and escape what goes into SDL ([7db3382](https://github.com/salamaashoush/ferrimock/commit/7db33823cd278c778ff374b740214fa50b881af6))
- *(lint)* Satisfy the spell check CI runs ([65d8fef](https://github.com/salamaashoush/ferrimock/commit/65d8fef7c656052e7cc8a357e1c43a0314eb1fda))
- *(test)* Gate the template entity tests on the feature that loads their schema ([aae3943](https://github.com/salamaashoush/ferrimock/commit/aae39437b789c41b6f080e1613d3f7cfd24fa2f9))
- *(world)* One relation, one mechanism, one answer ([95ed26b](https://github.com/salamaashoush/ferrimock/commit/95ed26ba90e248551936feb77caf85ef2a2dac83))
- *(world)* Hierarchies get roots, levels, and a cascade that reaches bottom ([385a5de](https://github.com/salamaashoush/ferrimock/commit/385a5deaa461ba0e187f008cce35f4be1757dc58))
- *(fake-data)* Draw a moment, not a year and a month and a day ([25d2477](https://github.com/salamaashoush/ferrimock/commit/25d2477da6711dbd0c06d8e3029916efbc895fc5))
- *(world)* A replacement replaces, and a token means what its field says ([dde3232](https://github.com/salamaashoush/ferrimock/commit/dde323298d83a872cbda8ede6fd9ec48f89fcf61))
- *(graphql)* A write whose key rides in an input object is still a write ([e6872da](https://github.com/salamaashoush/ferrimock/commit/e6872dafe968574d9e4559eb093dc6cf05987f18))
- *(rest)* Answer with the shape the document declared ([86f2284](https://github.com/salamaashoush/ferrimock/commit/86f228428c9f3551e5b4e29b99e35c638abc4dbc))
- *(lint)* Clear the two lints --all-features surfaces ([1a3081f](https://github.com/salamaashoush/ferrimock/commit/1a3081f09ccfe59f2488b3abf94d689747c6617d))
- *(features)* Fake-data needs the type detector it generates for ([f7bfd96](https://github.com/salamaashoush/ferrimock/commit/f7bfd9685d06f135b4328284bb7cf0ff739de4bc))
- *(deps)* Take h2 0.4.18 for RUSTSEC-2026-0258 ([5812e67](https://github.com/salamaashoush/ferrimock/commit/5812e6719e919f4acd08aaed4699a99b6ed4bddd))
- *(graphql)* A non-null lookup that misses says what is missing ([7417062](https://github.com/salamaashoush/ferrimock/commit/7417062244d32d69a3099f9747a438bc36ae0077))
- *(npm)* Resolve against the public registry, and pin it in the repo ([e277f30](https://github.com/salamaashoush/ferrimock/commit/e277f30013d5cc6e55d934fe88075fbda070abda))
- *(napi)* Hold @napi-rs/cli at 3.6, whose type generator still works ([75226c2](https://github.com/salamaashoush/ferrimock/commit/75226c273f3488f17fe7f5f55215d7bc97deac1f))

### Build

- Enable LTO and a single codegen unit for release ([5952fac](https://github.com/salamaashoush/ferrimock/commit/5952facb5c60e059524b75f643f058a8a0c256dc))
- *(napi)* Upgrade to napi 3.12.1 / napi-derive 3.6.3 ([b35492c](https://github.com/salamaashoush/ferrimock/commit/b35492c2c9a921f82aeca3cd404f3a59454ea669))

### Documentation

- OpenAPI-derived backends ([9728501](https://github.com/salamaashoush/ferrimock/commit/9728501f926b355425057a8479bb22d2a1570cff))
- Resolve three intra-doc links ([052a7a7](https://github.com/salamaashoush/ferrimock/commit/052a7a7b969563b545ef9629c23b513cbf16c080))
- *(graphql)* Point at the command that exists ([e06b554](https://github.com/salamaashoush/ferrimock/commit/e06b554026c3a4ddca7e85e865967f38515d5d6e))
- What to do about the nine breaking changes in 0.3.0 ([c3acf42](https://github.com/salamaashoush/ferrimock/commit/c3acf422c711b65a6301529b0e7d6870fb1ff444))

### Features

- *(fake-data)* Seed every generator for reproducible runs ([2929a99](https://github.com/salamaashoush/ferrimock/commit/2929a99ee4f638b95d22b59159fcc96f974a8f55))
- *(engine)* Explain why a request matched, or did not ([4cce68b](https://github.com/salamaashoush/ferrimock/commit/4cce68bfcebac66a609e70b17783aa8276b8393c))
- *(engine)* Count matches so tests can assert what ran ([94ce7b6](https://github.com/salamaashoush/ferrimock/commit/94ce7b68f5f9a42e21ee0d1234217d344615b593))
- *(config)* Declarative network_error ([60d3c34](https://github.com/salamaashoush/ferrimock/commit/60d3c346bbaba84b04ad16afe78b4f7af88361b8))
- *(engine)* Say what to widen when a request finds no mock ([7faa1a7](https://github.com/salamaashoush/ferrimock/commit/7faa1a7910bc464e38bf75a988f130a304afca9f))
- *(engine)* Rank near misses by shared path, not registration order ([0f59bb0](https://github.com/salamaashoush/ferrimock/commit/0f59bb03da4e3da8f80efdda45da17575fc4db45))
- *(config)* Retire a mock after one match with `once` ([07e847f](https://github.com/salamaashoush/ferrimock/commit/07e847f84905d5862ab3191ff026d80d76d7f3e8))
- *(consolidation)* Measure fidelity, and make the judgement calls pluggable ([6374e64](https://github.com/salamaashoush/ferrimock/commit/6374e64b2b300e3b48a80a075b2910131f74321e))
- *(ml)* Fit a network, and measure whether it was worth it ([d8ea8ae](https://github.com/salamaashoush/ferrimock/commit/d8ea8ae4ccd4ce065cecb4b62c582f71d5c973f2))
- *(fake-data)* Generate a value in the shape the field wrote it ([c219ded](https://github.com/salamaashoush/ferrimock/commit/c219dedc92cabb014d856482feb12e4cad5377a8))
- *(codegen)* Answer with the value the request carried, wherever it sits ([2ac32f9](https://github.com/salamaashoush/ferrimock/commit/2ac32f94e1d8b0069cb479f582ae704d4987c5e9))
- *(consolidation)* Detect request variance everywhere, and generalize a lone recording ([258682e](https://github.com/salamaashoush/ferrimock/commit/258682e8cb8250349352aa95b608fba60524fe30))
- *(cli)* A flag to generalize lone recordings, and leaf-level fidelity ([e7f344b](https://github.com/salamaashoush/ferrimock/commit/e7f344bd34c2cee7ef30178f7b6610d41b817908))
- *(ml)* An offline oracle for what the detector gets wrong ([d826892](https://github.com/salamaashoush/ferrimock/commit/d8268928747ec4485886b067060bfbc3e5173b8e))
- *(spec)* Compile a GraphQL schema into a stateful backend ([10ef003](https://github.com/salamaashoush/ferrimock/commit/10ef003adffe45472f0703391fbf9015aa19236a))
- *(cli)* Serve a spec as a backend, and say how much of it is real ([e0f6572](https://github.com/salamaashoush/ferrimock/commit/e0f657280e4f5d7eb17468a98286673e8a595e47))
- *(spec)* Read an OpenAPI document into the entity world ([5ab1e9c](https://github.com/salamaashoush/ferrimock/commit/5ab1e9c2b2efcd296675ac721e438f7e12ec1e6b))
- *(spec)* Serve an OpenAPI document as one mock per operation ([dd68c5b](https://github.com/salamaashoush/ferrimock/commit/dd68c5bdde85bde10619f0f227cbac6a074483a3))
- *(har)* Let a deployment name its own infrastructure headers ([7a70ece](https://github.com/salamaashoush/ferrimock/commit/7a70ece4a2cdb319fce6318b66cccc4e18cb22ff))
- *(world)* Field overrides, values worth reading, and one link per relation ([e74f8f9](https://github.com/salamaashoush/ferrimock/commit/e74f8f9bbcfe4d0da021f2b0dbbe9644c5cf422e))
- *(world)* A doctor that names the world's own tells ([8ded00f](https://github.com/salamaashoush/ferrimock/commit/8ded00f9efb95c8aed35aa80bdc32657c123bd29))
- *(world)* Size the world from the shape of the graph ([5ccf29d](https://github.com/salamaashoush/ferrimock/commit/5ccf29d80e7ae8de8bf32a03bab1b96deed6d2c0))
- *(world)* Split required from nullable, and let a field be missing ([18aec3c](https://github.com/salamaashoush/ferrimock/commit/18aec3ce310726c5f9ba63aeb459b18ab0502625))
- *(world)* Give every value a distribution instead of a flat line ([b775807](https://github.com/salamaashoush/ferrimock/commit/b7758076206c18052d00e92e0bb6cf5427f83226))
- *(world)* Scatter the census so a partition stops being visible ([79c93fb](https://github.com/salamaashoush/ferrimock/commit/79c93fbbdc87cfa167af4f68c0ff3ee0699855dc))
- *(world)* Let a record's fields agree with each other ([4146d6e](https://github.com/salamaashoush/ferrimock/commit/4146d6e00775afb08eaee3b98f34316856e7b8a0))
- *(world)* Give the world a history, and ids that agree with it ([cb01373](https://github.com/salamaashoush/ferrimock/commit/cb0137397f3efecb89a2de9f8566afa33e2054b8))
- *(spec)* Read the values a document wrote for a field ([502871c](https://github.com/salamaashoush/ferrimock/commit/502871c63e8aec8babf2f6d0ba3936c3551b1e52))
- *(world)* A status is a position in a lifecycle, not a draw from a set ([b6738fe](https://github.com/salamaashoush/ferrimock/commit/b6738feb3531d41dc7a413a32e5d836a527fe321))
- *(world)* Put every record somewhere ([be2f6d4](https://github.com/salamaashoush/ferrimock/commit/be2f6d40375a2840f928cea850bccfbf1e7d035d))
- *(world)* Give a many-to-many a tail on both sides ([2d9f7c1](https://github.com/salamaashoush/ferrimock/commit/2d9f7c1f8f48b81a704d986fc2bbf1cf8e1d8ea5))
- *(spec)* /me is whoever asked, not record zero ([9cc7139](https://github.com/salamaashoush/ferrimock/commit/9cc7139c1901f529e3ec4a4fd5fb622867e55857))
- *(spec)* Let a mount ask for the behaviour a document cannot describe ([4231ef2](https://github.com/salamaashoush/ferrimock/commit/4231ef23d54948046db88ab09fb090c4f1a1df1a))
- *(spec)* Fit a world's parameters to a recording ([67b2409](https://github.com/salamaashoush/ferrimock/commit/67b2409190ba0e79985371b733f7f3913587abf2))
- *(world)* Start empty, and keep what was written across a restart ([8290d9a](https://github.com/salamaashoush/ferrimock/commit/8290d9ac4011576ec280031c71de6a7c3a66a511))

### Miscellaneous

- Use neutral example endpoints in tests ([648e52d](https://github.com/salamaashoush/ferrimock/commit/648e52d6ab5ab08db7d908ab61a72ab27e08f112))
- *(bench)* Keep benchmark runs out of the repo ([99f09ce](https://github.com/salamaashoush/ferrimock/commit/99f09cef155aa0020bc2905369bb255df3eb3d74))
- Generalize the vendor-specific examples ([5695f05](https://github.com/salamaashoush/ferrimock/commit/5695f057d5a1fc15b36f3ac3acb4a31a175cdd94))
- Bring the npm manifests up to the workspace version ([f1f47cf](https://github.com/salamaashoush/ferrimock/commit/f1f47cf2060916b8e2786f0ef386b035bd7c4521))
- *(napi)* Regenerate the loader for the new version ([2f639b9](https://github.com/salamaashoush/ferrimock/commit/2f639b9e84786677962bdf04ae5eeaacaba9774b))

### Performance

- *(napi)* Resolve fall-through chains in Rust, not across NAPI ([cc70649](https://github.com/salamaashoush/ferrimock/commit/cc70649b922e958c6e49b3d2190f3aeabc516883))
- *(engine)* Match a recorded query without allocating per candidate ([cbf55c9](https://github.com/salamaashoush/ferrimock/commit/cbf55c9f5bdfbfe20ea934e25485198deea0823c))
- *(world)* Hash a stream name instead of building one ([00d557c](https://github.com/salamaashoush/ferrimock/commit/00d557cce1efe407824140046d62661f44b8cb94))
- *(world)* Settle a record's derived fields in one pass ([385daab](https://github.com/salamaashoush/ferrimock/commit/385daab860cb65f41cfb6aa1ef0b18d42f480b17))
- *(doctor)* Cap what a check walks by what it needs ([4448322](https://github.com/salamaashoush/ferrimock/commit/4448322ae052df00650f93c2f3158505835d9819))

### Refactoring

- *(napi)* Take filesystem paths as PathBuf, not String ([0b3e36b](https://github.com/salamaashoush/ferrimock/commit/0b3e36b0aaa1a3b15678ed73fced520995522b1f))
- *(engine)* Resolve a native handler in Rust, and stop it slowing matching ([b945b54](https://github.com/salamaashoush/ferrimock/commit/b945b5427c46db1c3f1b23eeb7c6c766471f3831))
- *(core)* Move the entity world into core and reach it from every lane ([225308f](https://github.com/salamaashoush/ferrimock/commit/225308f79957999de8c0b8b087ddb0dcd1af7f81))
- *(spec)* Move RootPlan to a protocol-neutral home ([302a787](https://github.com/salamaashoush/ferrimock/commit/302a7870fd424eeded10de0b5d46eeec05ab7d26))

### Testing

- *(bench)* Measure ferrimock against MSW fairly, and correct the claims ([acfa02d](https://github.com/salamaashoush/ferrimock/commit/acfa02d920d44296526b43d4782a7fc2a1d9a2f8))
- *(bench)* Replay a real recording corpus, and measure at that scale ([c834c23](https://github.com/salamaashoush/ferrimock/commit/c834c2363952dfc33ea37e7f3b27969c0f4bf886))
- Cover replay fidelity and the properties consolidation must hold ([41a0a28](https://github.com/salamaashoush/ferrimock/commit/41a0a28b0a72a2564019671fb047146134b21227))
- Stop two suites measuring the machine rather than the code ([200c63e](https://github.com/salamaashoush/ferrimock/commit/200c63e1297ffe764397ef1e62860541ffff7e9c))
- *(world)* Drive persistence through a restart, not a round trip ([1733f0a](https://github.com/salamaashoush/ferrimock/commit/1733f0ada05ddf8409e400ba31f29a7ced319a5c))

## [0.2.0] - 2026-08-07

### Features

- Upgrade to Tera 2, replace serde_yaml, decouple scripting from Tera ([555d81d](https://github.com/salamaashoush/ferrimock/commit/555d81de3533004b603eb75ecc170d5108f48932))

### Miscellaneous

- Release v0.2.0 ([b41ef00](https://github.com/salamaashoush/ferrimock/commit/b41ef008f3732c36f2bde421ba56ab91714410df))

### Performance

- *(template)* Give each cached template its own Tera instance ([0e3aadc](https://github.com/salamaashoush/ferrimock/commit/0e3aadce906df2d99e924eb379ccf644b3e0acd0))

## [0.1.7] - 2026-07-10

### Bug Fixes

- *(npm)* Pin workspace-internal dependencies to real versions ([a9ec78a](https://github.com/salamaashoush/ferrimock/commit/a9ec78a058c9351e4132ad0a956700e7391427a1))

