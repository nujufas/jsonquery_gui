# scoop-jsonquery-gui

[Scoop](https://scoop.sh) bucket for [jsonquery gui](https://github.com/nujufas/jsonquery_gui), a fast,
native desktop GUI for browsing and querying large JSON files with jq, JSONPath, JMESPath and JSON Pointer.

[![Tests](https://github.com/{{REPO}}/actions/workflows/ci.yml/badge.svg)](https://github.com/{{REPO}}/actions/workflows/ci.yml)
[![Excavator](https://github.com/{{REPO}}/actions/workflows/excavator.yml/badge.svg)](https://github.com/{{REPO}}/actions/workflows/excavator.yml)

## Install

```pwsh
scoop bucket add jsonquery-gui https://github.com/{{REPO}}
scoop install jsonquery-gui/jsonquery-gui
```

Then start it from the Start menu (`jsonquery gui`) or with `jsonquery-gui` in a terminal.
Update with `scoop update jsonquery-gui`.

The app is a single portable executable and is not code-signed, so Windows SmartScreen may
warn the first time it runs.

## How this bucket is kept current

The manifest in `bucket/` has `checkver` and `autoupdate` entries, so the Excavator workflow
(every four hours) picks up each new release of jsonquery gui and commits the new version and
hash. Bugs in the app belong in the
[main project](https://github.com/nujufas/jsonquery_gui/issues); problems with this manifest
(a failed install, a hash error) belong here.
