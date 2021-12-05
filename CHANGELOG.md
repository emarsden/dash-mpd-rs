# Changelog

## [0.2.0] - 2021-12-XX
### Changed
- On platforms that support extended filesystem attributes, write the origin MPD URL to attribute
  `user.xdg.origin.url` on the output media file, using the `xattr` crate, unless the URL contains
  sensitive information such as a password. Likewise, write any MPD.ProgramInformation.{Title,
  Source, Copyright} information from the MPD manifest to attributes user.dublincore.{title, source,
  rights}.
- The `serviceLocation` attribute on `BaseURL` nodes is now public.
- Downloading: improve handling of transient HTTP errors.

## [0.1.0] - 2021-12-01

- Initial release
