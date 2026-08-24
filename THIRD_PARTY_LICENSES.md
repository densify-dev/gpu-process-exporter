# Third-party licenses

This file lists the Go modules used to build GPU Process Exporter and the license notices found in those modules. The release image also contains Debian trixie-slim runtime packages, including `ca-certificates`; use the release SBOM for the exact package set and versions in a built image.

Generated from `go list -m -json all`. Local paths are intentionally omitted.

## Go modules

| Module | Version |
| --- | --- |
| `github.com/densify-dev/gpu-process-exporter` | `(main module)` |
| `cloud.google.com/go/compute/metadata` | `v0.3.0` |
| `github.com/NVIDIA/go-nvml` | `v0.13.0-1` |
| `github.com/NYTimes/gziphandler` | `v1.1.1` |
| `github.com/alecthomas/kingpin/v2` | `v2.4.0` |
| `github.com/alecthomas/units` | `v0.0.0-20240927000941-0f3dac36c52b` |
| `github.com/beorn7/perks` | `v1.0.1` |
| `github.com/cespare/xxhash/v2` | `v2.3.0` |
| `github.com/creack/pty` | `v1.1.9` |
| `github.com/davecgh/go-spew` | `v1.1.2-0.20180830191138-d8f796af33cc` |
| `github.com/emicklei/go-restful/v3` | `v3.13.0` |
| `github.com/fxamacker/cbor/v2` | `v2.9.2` |
| `github.com/go-logr/logr` | `v1.4.3` |
| `github.com/go-openapi/jsonpointer` | `v0.23.1` |
| `github.com/go-openapi/jsonreference` | `v0.21.5` |
| `github.com/go-openapi/swag` | `v0.26.0` |
| `github.com/go-openapi/swag/cmdutils` | `v0.26.0` |
| `github.com/go-openapi/swag/conv` | `v0.26.0` |
| `github.com/go-openapi/swag/fileutils` | `v0.26.0` |
| `github.com/go-openapi/swag/jsonname` | `v0.26.0` |
| `github.com/go-openapi/swag/jsonutils` | `v0.26.0` |
| `github.com/go-openapi/swag/jsonutils/fixtures_test` | `v0.26.0` |
| `github.com/go-openapi/swag/loading` | `v0.26.0` |
| `github.com/go-openapi/swag/mangling` | `v0.26.0` |
| `github.com/go-openapi/swag/netutils` | `v0.26.0` |
| `github.com/go-openapi/swag/stringutils` | `v0.26.0` |
| `github.com/go-openapi/swag/typeutils` | `v0.26.0` |
| `github.com/go-openapi/swag/yamlutils` | `v0.26.0` |
| `github.com/go-openapi/testify/enable/yaml/v2` | `v2.4.2` |
| `github.com/go-openapi/testify/v2` | `v2.4.2` |
| `github.com/golang-jwt/jwt/v5` | `v5.3.0` |
| `github.com/golang/protobuf` | `v1.5.0` |
| `github.com/google/btree` | `v1.1.3` |
| `github.com/google/gnostic-models` | `v0.7.1` |
| `github.com/google/go-cmp` | `v0.7.0` |
| `github.com/google/gofuzz` | `v1.0.0` |
| `github.com/google/uuid` | `v1.6.0` |
| `github.com/gorilla/websocket` | `v1.5.4-0.20250319132907-e064f32e3674` |
| `github.com/josharian/intern` | `v1.0.0` |
| `github.com/jpillora/backoff` | `v1.0.0` |
| `github.com/json-iterator/go` | `v1.1.12` |
| `github.com/julienschmidt/httprouter` | `v1.3.0` |
| `github.com/klauspost/compress` | `v1.18.0` |
| `github.com/kr/pretty` | `v0.3.1` |
| `github.com/kr/text` | `v0.2.0` |
| `github.com/kylelemons/godebug` | `v1.1.0` |
| `github.com/mailru/easyjson` | `v0.7.7` |
| `github.com/moby/spdystream` | `v0.5.1` |
| `github.com/modern-go/concurrent` | `v0.0.0-20180306012644-bacd9c7ef1dd` |
| `github.com/modern-go/reflect2` | `v1.0.3-0.20250322232337-35a7c28c31ee` |
| `github.com/munnerz/goautoneg` | `v0.0.0-20191010083416-a7dc8b61c822` |
| `github.com/mwitkow/go-conntrack` | `v0.0.0-20190716064945-2f068394615f` |
| `github.com/mxk/go-flowrate` | `v0.0.0-20140419014527-cca7078d478f` |
| `github.com/peterbourgon/diskv` | `v2.0.1+incompatible` |
| `github.com/pmezard/go-difflib` | `v1.0.1-0.20181226105442-5d4384ee4fb2` |
| `github.com/prometheus/client_golang` | `v1.23.2` |
| `github.com/prometheus/client_model` | `v0.6.2` |
| `github.com/prometheus/common` | `v0.67.5` |
| `github.com/prometheus/procfs` | `v0.20.1` |
| `github.com/rogpeppe/go-internal` | `v1.14.1` |
| `github.com/spf13/pflag` | `v1.0.10` |
| `github.com/stretchr/objx` | `v0.5.2` |
| `github.com/stretchr/testify` | `v1.11.1` |
| `github.com/x448/float16` | `v0.8.4` |
| `github.com/xhit/go-str2duration/v2` | `v2.1.0` |
| `go.uber.org/goleak` | `v1.3.0` |
| `go.yaml.in/yaml/v2` | `v2.4.4` |
| `go.yaml.in/yaml/v3` | `v3.0.4` |
| `golang.org/x/crypto` | `v0.55.0` |
| `golang.org/x/mod` | `v0.38.0` |
| `golang.org/x/net` | `v0.58.0` |
| `golang.org/x/oauth2` | `v0.36.0` |
| `golang.org/x/sync` | `v0.22.0` |
| `golang.org/x/sys` | `v0.47.0` |
| `golang.org/x/term` | `v0.45.0` |
| `golang.org/x/text` | `v0.41.0` |
| `golang.org/x/time` | `v0.15.0` |
| `golang.org/x/tools` | `v0.48.0` |
| `golang.org/x/tools/go/expect` | `v0.1.0-deprecated` |
| `golang.org/x/tools/go/packages/packagestest` | `v0.1.1-deprecated` |
| `google.golang.org/protobuf` | `v1.36.12-0.20260120151049-f2248ac996af` |
| `gopkg.in/check.v1` | `v1.0.0-20201130134442-10cb98267c6c` |
| `gopkg.in/evanphx/json-patch.v4` | `v4.13.0` |
| `gopkg.in/inf.v0` | `v0.9.1` |
| `gopkg.in/yaml.v3` | `v3.0.1` |
| `k8s.io/api` | `v0.36.1` |
| `k8s.io/apimachinery` | `v0.36.1` |
| `k8s.io/client-go` | `v0.36.1` |
| `k8s.io/gengo/v2` | `v2.0.0-20250922181213-ec3ebc5fd46b` |
| `k8s.io/klog/v2` | `v2.140.0` |
| `k8s.io/kube-openapi` | `v0.0.0-20260512234627-ef417d054102` |
| `k8s.io/streaming` | `v0.36.1` |
| `k8s.io/utils` | `v0.0.0-20260507154919-ff6756f316d2` |
| `sigs.k8s.io/json` | `v0.0.0-20250730193827-2d320260d730` |
| `sigs.k8s.io/randfill` | `v1.0.0` |
| `sigs.k8s.io/structured-merge-diff/v6` | `v6.4.0` |
| `sigs.k8s.io/yaml` | `v1.6.0` |

## Notices found in Go modules

### github.com/densify-dev/gpu-process-exporter

```text
GPU Process Exporter
Copyright 2026 Evenkeel Inc. d/b/a Kubex. All rights reserved.

This product includes software developed by the Prometheus Authors.
This product includes software developed at SoundCloud Ltd.
```

### github.com/go-openapi/jsonpointer v0.23.1

```text
Copyright 2015-2025 go-swagger maintainers

// SPDX-FileCopyrightText: Copyright 2015-2025 go-swagger maintainers
// SPDX-License-Identifier: Apache-2.0

This software library, github.com/go-openapi/jsonpointer, includes software developed
by the go-swagger and go-openapi maintainers ("go-swagger maintainers").

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this software except in compliance with the License.

You may obtain a copy of the License at

  http://www.apache.org/licenses/LICENSE-2.0.

This software is copied from, derived from, and inspired by other original software products.
It ships with copies of other software which license terms are recalled below.

The original software was authored on 25-02-2013 by sigu-399 (https://github.com/sigu-399, sigu.399@gmail.com).

github.com/sigu-399/jsonpointer
===========================

// SPDX-FileCopyrightText: Copyright 2013 sigu-399 ( https://github.com/sigu-399 )
// SPDX-License-Identifier: Apache-2.0

Copyright 2013 sigu-399 ( https://github.com/sigu-399 )

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

  http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### github.com/go-openapi/jsonreference v0.21.5

```text
Copyright 2015-2025 go-swagger maintainers

// SPDX-FileCopyrightText: Copyright 2015-2025 go-swagger maintainers
// SPDX-License-Identifier: Apache-2.0

This software library, github.com/go-openapi/jsonreference, includes software developed
by the go-swagger and go-openapi maintainers ("go-swagger maintainers").

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this software except in compliance with the License.

You may obtain a copy of the License at

  http://www.apache.org/licenses/LICENSE-2.0.

This software is copied from, derived from, and inspired by other original software products.
It ships with copies of other software which license terms are recalled below.

The original software was authored on 25-02-2013 by sigu-399 (https://github.com/sigu-399, sigu.399@gmail.com).

github.com/sigh-399/jsonreference
===========================

// SPDX-FileCopyrightText: Copyright 2013 sigu-399 ( https://github.com/sigu-399 )
// SPDX-License-Identifier: Apache-2.0

Copyright 2013 sigu-399 ( https://github.com/sigu-399 )

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

  http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### github.com/go-openapi/testify/v2 v2.4.2

```text
Copyright 2025 go-swagger maintainers

// SPDX-FileCopyrightText: Copyright 2025 go-swagger maintainers
// SPDX-License-Identifier: Apache-2.0

This software library, github.com/go-openapi/testify, includes software developed
by the go-swagger and go-openapi maintainers ("go-swagger maintainers").

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this software except in compliance with the License.
You may obtain a copy of the License at

This software is copied from, derived from, and inspired mainly by github.com/stretchr/testify.
It ships with copies of other software which license terms are recalled below.


github.com/stretchr/testify
===========================

// SPDX-FileCopyrightText: Copyright (c) 2012-2020 Mat Ryer, Tyler Bunnell and contributors.
// SPDX-License-Identifier: MIT

MIT License

Copyright (c) 2012-2020 Mat Ryer, Tyler Bunnell and contributors.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

github.com/davecgh/go-spew
===========================

// SPDX-FileCopyrightText: Copyright (c) 2012-2016 Dave Collins <dave@davec.name>
// SPDX-License-Identifier: ISC

ISC License

Copyright (c) 2012-2016 Dave Collins <dave@davec.name>

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

github.com/pmezard/go-difflib
===========================

// SPDX-FileCopyrightText: Copyright (c) 2013, Patrick Mezard
// SPDX-License-Identifier: MIT-like

Copyright (c) 2013, Patrick Mezard
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

    Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
    Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.
    The names of its contributors may not be used to endorse or promote
products derived from this software without specific prior written
permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED
TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### github.com/prometheus/client_golang v1.23.2

```text
Prometheus instrumentation library for Go applications
Copyright 2012-2015 The Prometheus Authors

This product includes software developed at
SoundCloud Ltd. (http://soundcloud.com/).


The following components are included in this product:

perks - a fork of https://github.com/bmizerany/perks
https://github.com/beorn7/perks
Copyright 2013-2015 Blake Mizerany, Björn Rabenstein
See https://github.com/beorn7/perks/blob/master/README.md for license details.

Go support for Protocol Buffers - Google's data interchange format
http://github.com/golang/protobuf/
Copyright 2010 The Go Authors
See source code for license details.
```

### github.com/prometheus/client_model v0.6.2

```text
Data model artifacts for Prometheus.
Copyright 2012-2015 The Prometheus Authors

This product includes software developed at
SoundCloud Ltd. (http://soundcloud.com/).
```

### github.com/prometheus/common v0.67.5

```text
Common libraries shared by Prometheus Go components.
Copyright 2015 The Prometheus Authors

This product includes software developed at
SoundCloud Ltd. (http://soundcloud.com/).
```

### github.com/prometheus/procfs v0.20.1

```text
procfs provides functions to retrieve system, kernel and process
metrics from the pseudo-filesystem proc.

Copyright 2014-2015 The Prometheus Authors

This product includes software developed at
SoundCloud Ltd. (http://soundcloud.com/).
```

### go.yaml.in/yaml/v2 v2.4.4

```text
Copyright 2011-2016 Canonical Ltd.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### go.yaml.in/yaml/v3 v3.0.4

```text
Copyright 2011-2016 Canonical Ltd.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### gopkg.in/yaml.v3 v3.0.1

```text
Copyright 2011-2016 Canonical Ltd.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### sigs.k8s.io/randfill v1.0.0

```text
When donating the randfill project to the CNCF, we could not reach all the
gofuzz contributors to sign the CNCF CLA. As such, according to the CNCF rules
to donate a repository, we must add a NOTICE referencing section 7 of the CLA
with a list of developers who could not be reached.

`7. Should You wish to submit work that is not Your original creation, You may
submit it to the Foundation separately from any Contribution, identifying the
complete details of its source and of any license or other restriction
(including, but not limited to, related patents, trademarks, and license
agreements) of which you are personally aware, and conspicuously marking the
work as "Submitted on behalf of a third-party: [named here]".`

Submitted on behalf of a third-party: @dnephin (Daniel Nephin)
Submitted on behalf of a third-party: @AlekSi (Alexey Palazhchenko)
Submitted on behalf of a third-party: @bbigras (Bruno Bigras)
Submitted on behalf of a third-party: @samirkut (Samir)
Submitted on behalf of a third-party: @posener (Eyal Posener)
Submitted on behalf of a third-party: @Ashikpaul (Ashik Paul)
Submitted on behalf of a third-party: @kwongtailau (Kwongtai)
Submitted on behalf of a third-party: @ericcornelissen (Eric Cornelissen)
Submitted on behalf of a third-party: @eclipseo (Robert-André Mauchin)
Submitted on behalf of a third-party: @yanzhoupan (Andrew Pan)
Submitted on behalf of a third-party: @STRRL (Zhiqiang ZHOU)
Submitted on behalf of a third-party: @disconnect3d (Disconnect3d)
```
