# Sample MagicaVoxel .vox files (downloaded 2026-10-07 by Urano)

Sources (both MIT licensed; licence texts in ephtracy_LICENSE and dotvox_LICENSE):

- ephtracy_*.vox: https://github.com/ephtracy/voxel-model (the MagicaVoxel author's repository), files vox/character/{chr_cat,chr_knight,chr_sol}.vox.
- dotvox_*.vox: https://github.com/dust-engine/dot_vox, files src/resources/{axes,metal-material,not_a,single-voxel-with-material}.vox (parser test resources).

Inventory (magic, version, SIZE chunks, chunk ids):

| file | bytes | version | models (x,y,z) | chunks |
|---|---|---|---|---|
| ephtracy_chr_cat.vox | 2312 | 150 | 20x20x20 | MAIN SIZE XYZI (no RGBA: default palette) |
| ephtracy_chr_sol.vox | 1236 | 150 | 20x21x20 | MAIN SIZE XYZI (no RGBA) |
| ephtracy_chr_knight.vox | 2688 | 150 | 20x21x20 | MAIN SIZE XYZI RGBA |
| dotvox_single-voxel-with-material.vox | 28993 | 150 | 1x1x1 | same set |
| dotvox_metal-material.vox | 29101 | 150 | 3x3x3 | same set |
| dotvox_axes.vox | 44053 | 200 | 4 models: 3 of 40x40x40 and 1 of 32x32x32 | nTRN(14) nGRP(4) nSHP(10) LAYR(16) MATL(256) rCAM rOBJ NOTE |
| dotvox_not_a.vox | 4471 | (magic DOOD) | none | negative case |
cd686ce1fd0d66975d4fb97a20fff1fbb43763cec0cf4eb327248c5ce7690d2a  dotvox_axes.vox
7ed3b4debf4baa0e1d74e592c1dda60995c2207a8f74d903dce402cc653434fc  dotvox_metal-material.vox
2c9dccb89079a1b1a3f3e89b4e7da301015a268b4190f56f9a6d02df95c36d3a  dotvox_not_a.vox
ac1a606494036d445a6c67d15466f6711b06ea1236f9053cda1e75b593e91267  dotvox_single-voxel-with-material.vox
16377209391da8398be5763c5d37ff370ee39c6328bd861d177cc220434ce1ef  ephtracy_chr_cat.vox
455208399cc7f888629bf4715284ffb5608a0157342e1e8135f460788dddf6f4  ephtracy_chr_knight.vox
f008203a601be9edb0e62ead69f2ccaf9fe3db9607c23966d1a1a24abcf960cc  ephtracy_chr_sol.vox
