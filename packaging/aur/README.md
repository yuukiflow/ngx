# AUR package (`ngx-bin`)

Installs the static binary from a GitHub release, with man pages and shell completions.

To publish or update (needs an AUR account with your SSH key):

```
git clone ssh://aur@aur.archlinux.org/ngx-bin.git aur-ngx-bin
cp PKGBUILD aur-ngx-bin/ && cd aur-ngx-bin
# set pkgver to the release, then:
updpkgsums
makepkg --printsrcinfo > .SRCINFO
makepkg -si          # test-install locally
git add PKGBUILD .SRCINFO && git commit -m "ngx-bin $pkgver" && git push
```

`license` is `unknown` until the project picks a license.
