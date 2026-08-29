# Example themes

Four complete themes, each defined only in configuration — no code, no rebuild.

Copy one into `~/.config/maxgus/themes/` and name it:

```console
$ mkdir -p ~/.config/maxgus/themes
$ cp nord.kdl ~/.config/maxgus/themes/
```

```kdl
// ~/.config/maxgus/config.kdl
set theme="nord"
```

That is the whole of it. Everything in `themes/` is read at startup, so
`M-x load-theme` offers them too and you can switch without a restart.

| File | Name | Starts from | Palette |
|---|---|---|---|
| [`gruvbox.kdl`](gruvbox.kdl) | `gruvbox` | `maxgus-dark` | Warm retro, low contrast |
| [`nord.kdl`](nord.kdl) | `nord` | `maxgus-dark` | Cool arctic blues |
| [`dracula.kdl`](dracula.kdl) | `dracula` | `maxgus-dark` | High contrast, vivid |
| [`solarized-light.kdl`](solarized-light.kdl) | `solarized-light` | `maxgus-light` | Precision light |

Each names itself and says which built-in it `base=`s on, so any face it does
not set still has a sensible value — which is why a light theme must start from
`maxgus-light`.

To adjust one without editing it, put a `theme` block of the same name in
`config.kdl`; it wins face by face.

Every face name they use is listed in
[../configuration-reference.md](../configuration-reference.md#face-names); a
name that is not one of them is reported at startup rather than silently never
painting.
