# ARW preview-derived colour starting point

Sony ARW files with no usable camera colour matrix previously used the generic camera-RGB≈sRGB fallback. This can produce visibly muted, dark photographs. The engine now estimates a bounded 3×3 correction from a file's own embedded camera JPEG. The JPEG provides colour correspondences, not output pixels: decoding, demosaicing, white balance, highlight reconstruction and RAW editing still operate on the sensor data.

A fixed sensor proxy (at most 96×96) is used at every requested output size. The reference is decoded with DCT scaling and a 64-million-pixel input limit. Reference and sensor aspect ratios must agree within 2%; references with a different aspect ratio are rejected. Near-black, clipped and nonfinite samples are excluded. At least 256 usable pairs, enough coloured samples and a nonsingular, bounded-condition input covariance are required. Ridge regression is biased towards an exposure-only correction, rather than unconstrained colour amplification. Matrix coefficients are bounded.

One third of the usable pixels are held out from fitting. After the normal RAW tone shoulder, the correction must reduce linear-display squared error by at least 30%, with per-channel RMS error at most 0.075 (a provisional camera-look tolerance, not a colour-accuracy certification). Unusable, monochrome, mismatched, ill-conditioned or poor-fitting references retain the existing fallback. DNG matrices and other RAW formats are unchanged. Render cache version 5 invalidates previously rendered thumbnails and views.

This is a per-file camera-look estimate, not a measured per-camera spectral calibration. The embedded JPEG includes camera tone curves, picture styles, noise reduction and possibly local/lens adjustments; a global matrix cannot reproduce all of them. Similar-sized cropped or warped references are not guaranteed to be detected. Skin colour and highlights can still differ. A camera-matrix database and chart-based verification remain necessary. Do not advertise this as Lightroom colour parity or complete camera coverage.

Validation uses synthetic coloured scenes with a known matrix, monochrome/nonfinite/singular inputs, unrelated references and mismatched lengths. Five private Sony ILCE-6700 ARWs are checked locally; photographs and embedded JPEGs are not committed or uploaded. See the PR for the observed acceptance count and remaining differences.

## Local observations (Sony ILCE-6700)

The guarded estimate was accepted on all five private samples. Brightness and yellow clothing improved; skin still differs from the camera JPEG. This is why the change is proposed as a draft for colour-science review and broader camera verification.

A camera JPEG from the same capture, the previous RAW renderer and the fitted RAW renderer were exported at 1200 px with neutral settings and separately with Exposure +1 EV, Saturation +100 and Contrast +50. All three controls changed the previous RAW output. Mean absolute encoded-RGB differences from each source’s neutral rendering were:

| Source | Exposure +1 | Saturation +100 | Contrast +50 |
|---|---:|---:|---:|
| Camera JPEG | 0.1930 | 0.0642 | 0.0373 |
| RAW, generic fallback | 0.1399 | 0.0318 | 0.0315 |
| RAW, preview-derived estimate | 0.1628 | 0.0509 | 0.0706 |

These are response diagnostics on one image, not colour-accuracy or Lightroom-parity scores. The weaker saturation response starts from an already muted rendering; increasing saturation does not replace missing camera colour characterization. JPEG uses an identity tone map at neutral settings; RAW uses the scene-to-display shoulder, so identical slider values are not expected to produce identical changes. Edited RAW was also checked in the headless UI with the actual GPU renderer, after waiting for pending jobs: the loupe reported `source: render`, the histogram and displayed pixels changed, and Exposure +1 / Saturation +100 remained in the controls. No slider-no-op was reproduced in this sample.

A second comparison used five separately supplied same-capture JPEGs, including four with embedded Display P3 profiles. RAW-only changes left all five JPEG exports pixel-identical. Two supplied JPEGs have a narrower crop than the RAW; no pixel-by-pixel colour-accuracy claim is made across those different frames. On the supplied matching JPEG for the slider-response sample, the corresponding response figures were exposure 0.1952, saturation 0.0517 and contrast 0.0260. RAW saturation response increased from 0.0318 to 0.0509 with the estimate.

**Unresolved white-balance limitation:** the generic camera model also derives the RAW as-shot temperature/tint without a measured camera matrix. The sample reports roughly 6829 K / −127 tint. Changing temperature alone to 9000 K causes a strong pink cast, even after preview-derived colour fitting. The estimate does not establish a physically calibrated illuminant or repair absolute-Kelvin WB behaviour. It must not be presented as a complete RAW-development fix; measured camera characterization and white-balance validation are still required.

For the colour-fit sample, independent CLI renders at 400 px and full resolution selected exactly the same fitted matrix. Full cold render/export took 3.96 s; 400 px took 0.48 s on this local machine (includes process/import/render/encode, not just the adjustment pipeline).
