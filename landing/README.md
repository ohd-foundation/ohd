# OHD landing page

Static landing page served on `ohd.dev`. `openhealthdata.org` 301-redirects to it.

## What's here

```
landing/
├── index.html       # the page
├── styles.css       # styles (dark default, light toggle)
├── theme.js         # minimal theme toggle (localStorage + prefers-color-scheme)
└── public/
    ├── favicon.svg
    └── og-image.png # social preview (TODO: design)
```

No build step. Plain HTML + CSS + ~30 LOC of vanilla JS. Loads instantly.

## Local preview

```bash
cd landing
python3 -m http.server 8080
# open http://localhost:8080/
```

Or any other static file server.

## Deploy (Caddy on Hetzner)

The deploy agent (`DEPLOYMENT.md` at the project root) copies this directory to `/var/www/ohd-landing` on the Hetzner box. Caddy serves it on the apex of `ohd.dev` with auto-TLS via Let's Encrypt:

```caddy
ohd.dev, www.ohd.dev {
    root * /var/www/ohd-landing
    file_server
    encode gzip zstd
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains; preload"
        X-Content-Type-Options nosniff
        Referrer-Policy no-referrer-when-downgrade
        Permissions-Policy "interest-cohort=()"
    }
    handle /downloads/* {
        # APK release artifacts. For now, redirect to GitHub Releases until
        # we ship a CI release pipeline. The deploy agent wires this.
        redir https://github.com/ohd-foundation/ohd/releases/latest 302
    }
}

openhealthdata.org, www.openhealthdata.org {
    redir https://ohd.dev{uri} 301
}
```

## APK download

The Hero CTA and the "Beta — Android" section both link to `/downloads/ohd-connect-latest.apk`. Until a CI release pipeline ships, Caddy redirects `/downloads/*` to GitHub Releases (see Caddyfile snippet above). When CI lands, swap the `redir` for a `file_server` rooted at `/var/www/ohd-downloads`.

## Editing the page

The page is single-file: `index.html`. Sections are commented; copy lives inline. Refer to `../ux-design.md` for palette + typography intent. Tagline is in the hero — change it there if positioning evolves.

## License

Apache-2.0 OR MIT, same as the rest of the OHD project.
