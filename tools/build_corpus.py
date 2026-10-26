#!/usr/bin/env python3
"""Builds tests/corpus from tools/kitchen_sink.xml.in.

* valid/kitchen-sink.scene.xml — the template with SHA-256 digests filled in;
* valid/*.scene.xml — small documents for version and edge semantics;
* invalid/<case>.scene.xml — one mutation of the kitchen sink per assert id
  and per structural/asset code, and a document per error of the engine's own
  rules, headed by `<!-- expect: CODES -->`;
* manifest.json — file -> expected codes.

Every document's XSD and Schematron verdict is checked against lxml
(tools/oracle.py). Asset codes (A01–A07) are outside the oracle's scope.
"""
import hashlib, json, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from oracle import verdict

TOOLS = Path(__file__).parent
CORPUS = TOOLS.parent / "tests" / "corpus"
MEDIA = CORPUS / "media"

base = re.sub(r"\{sha:([^}]+)\}", lambda m: hashlib.sha256((MEDIA / m.group(1)).read_bytes()).hexdigest(),
              (TOOLS / "kitchen_sink.xml.in").read_text())
MINIMAL = '<scene version="1.1"><project width="1920" height="1080" fps="30" duration="5"/><composition/></scene>\n'
VOLUME = '''<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/>
<assets><volume id="smoke" src="../media/uniform.srvol" boundsMinX="0" boundsMinY="0" boundsMinZ="0" boundsMaxX="32" boundsMaxY="32" boundsMaxZ="2"/></assets>
<composition><object3D id="cloud" primitive="volume" volume="smoke"><medium albedo="#000000" emissionColor="#FF4000" emissionScale="0.2"/></object3D></composition></scene>\n'''
VOLUME_SEQUENCE = VOLUME.replace('src="../media/uniform.srvol"',
    'src="../media/uniform-%02d.srvol" first="0" last="0" fps="24000/1001" interpolation="linear"')

VOLUME_BAKED = VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/baked-volume/manifest.srvseq" format="srvseq" sha256="68cb87c6720ed0ac07c87a7ae48f5a3a69c326c085f8dcb655be7738035432d8"')

PYRO = '<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/>\n<composition><object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource shape="sphere" radius="2" densityRate="2" start="0.15" end="0.35"/><pyroImpulse shape="box" width="2" height="2" depth="2" time="0.5" density="1" temperature="3000"/></pyro><medium blackbody="true" emissionScale="0.1"/></object3D></composition></scene>\n'

PYRO_FOLLOW = PYRO.replace('<pyro ', '<pyro boundary="open" follow="true" followMargin="2" followLoss="0.000001" ')
VOXELS = '<scene version="1.3"><project width="32" height="32" fps="30" duration="1"/><assets><voxelAsset id="model" src="../media/voxels.vox" maxCells="100000"/><mesh id="shape" src="../media/robot.glb"/></assets><materials><material id="stone" baseColor="#808080"/><material id="moss" baseColor="#406040"/></materials><composition><object3D id="build" primitive="voxels" voxels="model" cellSize="2" palette="stone moss" surface="blocks"/></composition></scene>\n'
VOXELS_FROM_MESH = VOXELS.replace('<voxelAsset id="model" src="../media/voxels.vox" maxCells="100000"/>', '<voxelAsset id="model" fromMesh="shape" cellSize="0.5"/>').replace(' palette="stone moss"', ' palette="file"')

PYRO_BLAST = PYRO.replace('<pyro ', '<pyro boundary="open" ').replace('</pyro>', '<pyroBlast time="0.3" energy="1000000" x="1" y="-1"/></pyro>')
# The block of cells is block.vox (16 x 4 x 16 cells of 0.25 m: 4 m by 1 m by 4 m, its origin at its minimum corner) and the ball falls at x = 2, z = 2: the middle of that block. The one
# document that scales the ground by 3 has a block of 12 m, and the ball is a third of the way across it, which is fine for what it checks (a scale written with a plus sign).
VOXEL_HEAD = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><assets><voxelAsset id="model" src="../media/block.vox" maxCells="100000"/></assets><materials><material id="stone" baseColor="#808080"/></materials><composition><object3D id="ball" primitive="sphere" radius="0.2" x="2" y="-8" z="2"><rigidBody mass="500" velocityY="120"/></object3D>'
VOXEL_TAIL = '</composition><physics pixelsPerMeter="1"/></scene>\n'
VOXEL_BODY = VOXEL_HEAD + '<object3D id="block" primitive="voxels" voxels="model" cellSize="0.25" material="stone"><rigidBody density="2400"/></object3D>' + VOXEL_TAIL
VOXEL_GROUND = VOXEL_HEAD + '<object3D id="ground" primitive="voxels" voxels="model" cellSize="0.25" material="stone" y="2"><rigidBody type="static" density="2400" maxFragments="128" fragmentMinCells="2" fragmentOverflow="dust" anchor="base"/><crater id="pit" source="ball" targetMaterial="softRock"/></object3D>' + VOXEL_TAIL
VOXEL_FRACTURE = VOXEL_HEAD + '<object3D id="block" primitive="voxels" voxels="model" cellSize="0.25" material="stone"><rigidBody density="2400" maxFragments="32" fragmentMinCells="1"/><fracture source="ball" pieces="8" seed="3"/></object3D>' + VOXEL_TAIL
VOXEL_STRESS = VOXEL_FRACTURE.replace('<fracture source="ball" pieces="8" seed="3"/>', '<fracture mode="stress" strength="2e6" pieces="8" seed="3"/>')
VOXEL_EJECTA = VOXEL_GROUND.replace(VOXEL_TAIL, '<particles3D id="debris" rate="0" gravityY="9.8" lifetime="3" maxParticles="500"><burst crater="pit"/></particles3D>' + VOXEL_TAIL)

PYRO_MESH = PYRO.replace("<composition>", '<assets><mesh id="source-mesh" src="../media/robot.glb"/></assets><composition>').replace('shape="sphere" radius="2"', 'shape="mesh" mesh="source-mesh"')

PYRO_COLLIDERS = PYRO.replace("<composition>", '<composition><object3D id="solid" primitive="box" width="2" height="2" depth="2"/>').replace("<pyro ", '<pyro colliders="solid" ')

SOLID_COLLIDERS = '<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><composition><object3D id="letters" primitive="text" text="O"/><object3D id="path" primitive="extrude" path="M0 0 L2 0 L2 2 Z"/><object3D id="clay" primitive="clay"><blob/></object3D><particles3D id="dust" colliders="letters path clay"/><object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="letters path clay"/></object3D></composition></scene>\n'

PARTICLES3D = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="floor" primitive="box" y="20" width="64" height="1" depth="64"/><particles3D id="dust" rate="0" speed="5" spread="120" gravityY="8" colliders="floor" lifetime="2"><burst time="0" count="12"/></particles3D></composition></scene>\n'

OCEAN = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2"><waterImpulse time="0.3" radius="2" amplitude="0.1"/><wave wavelength="4" amplitude="0.1" phase="0"/></ocean></composition></scene>\n'

OCEAN_COUPLED = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="seabed" primitive="plane" width="40" height="40" segments="8" y="2" rotationX="-90"><crater radius="3" depth="1" rimHeight="0.2" rimWidth="1" start="0.5" end="1"/></object3D><object3D id="rock" primitive="sphere" radius="1" y="-3"/><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2" colliders="seabed rock"/></composition></scene>\n'

GLOBE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><tiles id="dem" src="../media/terrain.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem" terrainTileSize="2" terrainZoom="0" planetRadius="1000" radius="20" x="32" y="32" segments="32"/></composition></scene>\n'

MORPH = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="set" src="../media/robot.glb"/></assets><composition><object3D id="piece" primitive="mesh" mesh="set" x="32" y="32"><morph name="smile" weight="0.8"><animate property="weight"><key time="0" value="0"/><key time="1" value="1"/></animate></morph></object3D></composition></scene>\n'
JOINT = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="set" src="../media/robot.glb"/></assets><composition><shape id="t" shape="rect" x="10" y="10" width="4" height="4" fill="#FFFFFF"/><object3D id="piece" primitive="mesh" mesh="set" x="32" y="32"><joint name="head" rotation="12" lookAt="t" lookAxis="z" influence="0.8" maxAngle="35"><animate property="rotationX"><key time="0" value="0"/><key time="1" value="10"/></animate></joint></object3D></composition></scene>\n'
STROKE_TEXT = '<scene version="1.2"><project width="320" height="120" fps="24" duration="2"/><assets><strokeFont id="hand" src="../media/test.jhf"/></assets><composition><shape id="w" shape="stroke-text" text="HI AB" strokeFont="hand" fontSize="40" width="300" height="80" stroke="#FFFFFF" strokeWidth="3" strokeCap="round" trimMode="sequential"><animate property="trimEnd"><key time="0" value="0"/><key time="1" value="1"/></animate></shape></composition></scene>\n'
PINNED_FONTS = ('<scene version="1.2"><project width="64" height="64" fps="10" duration="1" fontPolicy="pinned"/>'
    '<assets><font id="inter" src="../media/inter.ttf" family="Inter" sha256="' + hashlib.sha256((MEDIA / "inter.ttf").read_bytes()).hexdigest() + '"/>'
    '<text id="t" text="Hi" width="60" height="30" size="20" font="Inter" fallback="Inter"/></assets>'
    '<composition><layer id="l" asset="t"/></composition></scene>\n')
MODEL_SELECT = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="set" src="../media/robot.glb"/></assets><materials><material id="clay" baseColor="#C48A5A"/></materials><composition><object3D id="piece" primitive="mesh" mesh="set" node="Knight" materialOverride="stone:clay old:clay" x="32" y="32"/></composition></scene>\n'
TEXT3D = '<scene version="1.1"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="title" primitive="text" text="HI" height="20" tracking="120" x="32" y="32"/></composition></scene>\n'
MESH_SEQUENCE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><assets><meshSequence id="frames" src="../media/mesh-frame-%d.obj" first="0" last="1" fps="1"/></assets><composition><object3D id="cache" primitive="mesh" mesh="frames" x="32" y="32"/></composition></scene>\n'

FRACTURE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><materials><material id="interior" baseColor="#A06030"/></materials><composition><object3D id="rock" primitive="box" width="4" height="4" depth="4"><rigidBody mass="8"/><fracture at="1" pieces="8" seed="18446744073709551615" interiorMaterial="interior" radialImpulse="4"/></object3D></composition></scene>\n'

POINTS = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><composition><repeat id="r" x="32" y="32"><points type="grid" columns="3" rows="2" spacingX="10" spacingY="10"><animate property="spacingX"><key time="0" value="5"/><key time="1" value="10"/></animate></points><shape id="s" shape="rect" width="4" height="4" fill="#FFFFFF"/></repeat></composition></scene>\n'

CONNECTOR = '<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><text id="note" text="Hi" width="20" height="10" size="8"/></assets><composition><shape id="a" shape="rect" x="4" y="4" width="10" height="10" fill="#FF0000"/><shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/><connector id="c" from="a" to="b" route="orthogonal" stroke="#00FF00" strokeWidth="2" markerEnd="arrow" label="note"><animate property="trimEnd"><key time="0" value="0"/><key time="1" value="1"/></animate></connector></composition></scene>\n'

LOGO_SHA = hashlib.sha256((MEDIA / "logo.png").read_bytes()).hexdigest()
PDF = ('<scene version="1.2"><project width="64" height="64" fps="24" duration="2"/><assets><pdf id="paper" src="../media/paper.pdf" '
       f'sha256="{"0" * 64}" dpi="72" cache="../media/logo.png" cacheSha256="{LOGO_SHA}" width="16" height="16">'
       '<region id="w" text="word" x="2" y="3" width="8" height="4"/></pdf></assets><composition><layer id="page" asset="paper" x="8" y="8"/>'
       '<shape id="mark" shape="ellipse" region="w" regionLayer="page" regionPadding="1" stroke="#FF0000" strokeWidth="1"/></composition></scene>\n')

CRATER = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition><object3D id="ground" primitive="plane" width="20" height="20" segments="40"><crater radius="4" depth="3" rimWidth="1" rimHeight="0.5" start="1" end="2"/><rigidBody type="static"/></object3D></composition></scene>\n'

CRATER_IMPACT = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition><object3D id="rock" primitive="sphere" radius="1" y="-8"><rigidBody mass="5"/></object3D><object3D id="ground" primitive="plane" width="20" height="20" segments="40" y="2"><crater id="pit" source="rock" targetMaterial="softRock"/><rigidBody type="static"/></object3D></composition><physics pixelsPerMeter="1"/></scene>\n'

PYRO_CRATER = CRATER_IMPACT.replace('</composition>', '<object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource crater="pit"/><pyroImpulse crater="pit" heatFraction="0.2"/></pyro></object3D></composition>')

OCEAN_BUOYANCY = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="float" primitive="sphere" radius="0.5" y="-1"><rigidBody shape="sphere" mass="200"/></object3D><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="4" colliders="float" bodyCoupling="buoyancy" bodyDrag="1.5"/></composition><physics pixelsPerMeter="1"/></scene>\n'
EJECTA_CRATER = CRATER_IMPACT.replace('</composition>', '<particles3D id="debris" rate="0" gravityY="9.8" lifetime="3" maxParticles="500"><burst crater="pit" count="200"/></particles3D></composition>')

OCEAN_ENTRY = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><object3D id="rock" primitive="sphere" radius="1" y="-4"><rigidBody mass="5"/></object3D><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="2" colliders="rock"><waterImpulse source="rock"/></ocean></composition><physics pixelsPerMeter="1"/></scene>\n'

def sub(old, new, count=1):
    def f(t):
        assert old in t, f"mutation anchor not found: {old!r}"
        return t.replace(old, new, count)
    return f

def chain(*fs):
    def f(t):
        for g in fs:
            t = g(t)
        return t
    return f

def ins_comp(xml):  # insert nodes at the start of <composition>
    return sub("<composition>\n", "<composition>\n    " + xml + "\n")

def v10(body, extra_sections=""):
    return f'<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/>{extra_sections}<composition>{body}</composition></scene>\n'

BLACKHOLE = '<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition><shape id="sky" shape="rect" x="0" y="0" width="64" height="64" fill="#000000"/><camera id="eye" x="0" y="0" z="-60" geodesics="true"/><blackHole id="hole" mass="1" x="0" y="0" z="0"/><accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/></composition></scene>\n'
FRACTURE_CONTACT = '<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><materials><material id="interior" baseColor="#A06030"/></materials><composition><object3D id="ball" primitive="sphere" radius="1" x="-8"><rigidBody mass="1"/></object3D><object3D id="rock" primitive="box" width="4" height="4" depth="4"><rigidBody mass="8"/><fracture source="ball" pieces="8" seed="3" interiorMaterial="interior"/></object3D></composition></scene>\n'

# (name, expected codes, transform-or-document, asset codes the oracle cannot see)
CASES = [
    ("solid-colliders-boil", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('primitive="clay"', 'primitive="clay" boil=" +12 " fingerprints=" +0.5 "')),
    ("solid-colliders-blob", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('<blob/>', '<blob><animate property="x"><key time="0" value="2"/></animate></blob>')),
    ("frx5-source-static", ["FRX5"], lambda _: FRACTURE_CONTACT.replace('<rigidBody mass="1"/>', '<rigidBody type="static"/>')),
    ("frx5-source-self", ["FRX5"], lambda _: FRACTURE_CONTACT.replace('source="ball"', 'source="rock"')),
    ("frx6-timed", ["FRX6"], lambda _: FRACTURE_CONTACT.replace('<fracture ', '<fracture at="1" ')),
    ("frx6-push", ["FRX6"], lambda _: FRACTURE_CONTACT.replace('<fracture ', '<fracture radialImpulse="4" ')),
    ("frx7-orphan", ["FRX7"], lambda _: FRACTURE.replace('<fracture ', '<fracture energyFraction="0.3" ')),
    ("vox1-version", ["VOX1"], lambda _: VOXELS.replace('version="1.3"', 'version="1.2"')),
    ("vox2-both-sources", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/voxels.vox" fromMesh="shape" cellSize="1"')),
    ("vox2-no-source", ["VOX2"], lambda _: VOXELS.replace(' src="../media/voxels.vox"', '')),
    ("vox2-cellsize-with-src", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/voxels.vox" cellSize="1"')),
    ("vox2-model-of-mesh", ["VOX2"], lambda _: VOXELS_FROM_MESH.replace('fromMesh="shape"', 'fromMesh="shape" model="0"')),
    ("vox2-grid-of-vox", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/voxels.vox" voxelGrid="voxels"')),
    ("vox2-model-of-srvol", ["VOX2"], lambda _: VOXELS.replace('src="../media/voxels.vox"', 'src="../media/uniform.srvol" model="0"')),
    ("vox3-frommesh", ["VOX3"], lambda _: VOXELS_FROM_MESH.replace('fromMesh="shape"', 'fromMesh="stone"')),
    ("vox4-asset", ["VOX4"], lambda _: VOXELS.replace('voxels="model"', 'voxels="shape"')),
    ("vox4-no-asset", ["VOX4"], lambda _: VOXELS.replace(' voxels="model"', '')),
    ("vox5-orphan", ["VOX5"], lambda _: VOXELS.replace('primitive="voxels"', 'primitive="box"')),
    ("vox5-surface-memory-orphan", ["VOX5"], lambda _: VOXELS.replace('primitive="voxels"', 'primitive="box" surfaceMemoryMiB="64"')),
    ("vox6-palette", ["VOX6"], lambda _: VOXELS.replace('palette="stone moss"', 'palette="stone nothing"')),
    ("vox8-shape", ["VOX8"], lambda _: CRATER_IMPACT.replace('<rigidBody type="static"/>', '<rigidBody type="static" shape="voxels"/>')),
    ("vox9-density", ["VOX9"], lambda _: FRACTURE.replace('<rigidBody mass="8"/>', '<rigidBody mass="8" density="2400"/>')),
    ("vox10-no-density", ["VOX10"], lambda _: VOXEL_BODY.replace(' density="2400"', '')),
    ("vox10-mass", ["VOX10"], lambda _: VOXEL_BODY.replace('density="2400"', 'density="2400" mass="5"')),
    ("vox11-slots", ["VOX11"], lambda _: VOXEL_BODY.replace('density="2400"', 'density="2400" maxFragments="8"')),
    ("vox12-anchor", ["VOX12"], lambda _: VOXEL_BODY.replace('density="2400"', 'density="2400" anchor="base"')),
    ("vox13-scale", ["VOX13"], lambda _: VOXEL_GROUND.replace('y="2">', 'y="2" scaleX="2">')),
    ("vox15-mesh-collider", ["VOX15"], lambda _: VOXEL_GROUND.replace('type="static" density="2400"', 'type="static" shape="trimesh"').replace(' maxFragments="128" fragmentMinCells="2" fragmentOverflow="dust" anchor="base"', '').replace('y="2">', 'y="2" scaleX="2">')),
    ("vox15-fracture-box", ["VOX15"], lambda _: VOXEL_FRACTURE.replace('<rigidBody density="2400" maxFragments="32" fragmentMinCells="1"/>', '<rigidBody shape="box"/>')),
    ("vox14-both", ["VOX14"], lambda _: VOXEL_GROUND.replace('</object3D>' + VOXEL_TAIL, '<fracture source="ball" pieces="4"/></object3D>' + VOXEL_TAIL)),
    ("crt13-mantle", ["CRT13"], lambda _: VOXEL_GROUND.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true"')),
    ("crt14-curve", ["CRT14"], lambda _: VOXEL_GROUND.replace('targetMaterial="softRock"', 'targetMaterial="softRock" curve="linear"')),
    ("crt15-no-source", ["CRT15"], lambda _: VOXEL_GROUND.replace('<crater id="pit" source="ball" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("crt17-angle", ["CRT17"], lambda _: VOXEL_EJECTA.replace('<burst crater="pit"/>', '<burst crater="pit" angle="30" angleSpread="5"/>')),
    ("crt16-count", ["CRT16"], lambda _: VOXEL_EJECTA.replace('<burst crater="pit"/>', '<burst crater="pit" count="200"/>')),
    ("crt16-no-count", ["CRT16"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst crater="pit"/>')),
    ("frx8-interior", ["FRX8"], lambda _: VOXEL_FRACTURE.replace('<fracture ', '<fracture interiorMaterial="stone" ')),
    ("frx9-partition", ["FRX9"], lambda _: FRACTURE_CONTACT.replace('<fracture ', '<fracture partition="voronoi" ')),
    ("frx10-pieces", ["FRX10"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="labels" labels="material" pieces="8"')),
    ("frx11-planes", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0"')),
    ("frx11-no-partition", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'planes="1 0 0 4"')),
    ("frx11-zero-normal", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0 4 0 0 0 2"')),
    ("frx11-infinite", ["FRX11"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0 1' + '0' * 400 + '"')),
    ("frx13-no-strength", ["FRX13"], lambda _: VOXEL_STRESS.replace(' strength="2e6"', '')),
    ("frx13-strength-alone", ["FRX13"], lambda _: VOXEL_FRACTURE.replace('<fracture ', '<fracture strength="2e6" ')),
    ("frx14-static", ["FRX14"], lambda _: VOXEL_STRESS.replace('<rigidBody density="2400"', '<rigidBody type="static" density="2400"')),
    ("frx15-source", ["FRX15"], lambda _: VOXEL_STRESS.replace('<fracture ', '<fracture source="ball" ')),
    ("frx12-labels", ["FRX12"], lambda _: VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="labels"')),
    ("vox7-exclusion", ["VOX7"], lambda _: VOXELS.replace('surface="blocks"', 'surface="blocks" mesh="shape"')),
    ("frx1", ["FRX1"], lambda _: FRACTURE.replace('version="1.3"', 'version="1.2"')),
    ("frx2", ["FRX2"], lambda _: FRACTURE.replace('<rigidBody mass="8"/>', '')),
    ("frx3", ["FRX3"], lambda _: FRACTURE.replace('interiorMaterial="interior"', 'interiorMaterial="rock"')),
    ("frx4", ["FRX4", "W01"], lambda _: FRACTURE.replace('radialImpulse="4"', 'radialImpulse="4" impulseX="NaN"')),
    ("crt1", ["CRT1"], lambda _: CRATER.replace('version="1.3"', 'version="1.2"')),
    ("crt2", ["CRT2"], lambda _: CRATER.replace('<crater ', '<crater/><crater ')),
    ("crt3", ["CRT3"], lambda _: CRATER.replace('end="2"', 'end="0"')),
    ("crt4", ["CRT4"], lambda _: CRATER.replace('rimWidth="1"', 'rimWidth="5"')),
    ("bh1-version", ["BH1"], lambda _: BLACKHOLE.replace('version="1.3"', 'version="1.2"')),
    ("bh2-two-holes", ["BH2"], lambda _: BLACKHOLE.replace('<accretionDisk', '<blackHole id="second" mass="2"/><accretionDisk')),
    ("bh3-disk-target", ["BH3"], lambda _: BLACKHOLE.replace('blackHole="hole"', 'blackHole="eye"')),
    ("bh4-inside-isco", ["BH4"], lambda _: BLACKHOLE.replace('<accretionDisk id="disk"', '<accretionDisk id="disk" innerRadius="5"')),
    ("bh4-outer-not-beyond-inner", ["BH4"], lambda _: BLACKHOLE.replace('outerRadius="20"', 'outerRadius="6"')),
    ("bh5-geodesics-without-hole", ["BH5"], lambda _: BLACKHOLE.replace('<blackHole id="hole" mass="1" x="0" y="0" z="0"/>', '').replace('<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>', '')),
    ("bh6-other-object", ["BH6"], lambda _: BLACKHOLE.replace('<blackHole', '<object3D id="ball" primitive="sphere" radius="1"/><blackHole')),
    ("bh7-inside-photon-sphere", ["BH7"], lambda _: BLACKHOLE.replace('z="-60" geodesics', 'z="-2.5" geodesics')),
    ("bh8-two-cameras", ["BH8"], lambda _: BLACKHOLE.replace('<blackHole', '<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><blackHole')),
    ("bh6-fluid", ["BH6"], lambda _: BLACKHOLE.replace('<blackHole', '<ocean id="sea"/><blackHole')),
    ("bh6-particles", ["BH6"], lambda _: BLACKHOLE.replace('<blackHole', '<particles3D id="dust" rate="1"/><blackHole')),
    ("crt4-envelope", ["CRT4"], lambda _: CRATER.replace('start="1"', 'influenceDepth="4" start="1"')),
    ("crt5", ["CRT5"], lambda _: CRATER.replace('type="static"', 'type="dynamic"')),
    ("crt6", ["CRT6"], lambda _: CRATER_IMPACT.replace('<crater ', '<crater depth="3" ')),
    ("crt7-material", ["CRT7"], lambda _: CRATER_IMPACT.replace(' targetMaterial="softRock"', '')),
    ("crt7-orphan", ["CRT7"], lambda _: CRATER.replace('<crater ', '<crater strength="1000" ')),
    ("crt9-capture-without-source", ["CRT9"], lambda _: CRATER.replace('<crater ', '<crater capture="true" ', 1)),
    ("crt8-static", ["CRT8"], lambda _: CRATER_IMPACT.replace('<rigidBody mass="5"/>', '<rigidBody type="static"/>')),
    ("s09-crater-id", ["S09"], lambda _: CRATER_IMPACT.replace('id="pit"', 'id="ground"')),
    ("pyro9-closed", ["PYRO9"], lambda _: PYRO_FOLLOW.replace('boundary="open" ', '')),
    ("pyro10-orphan", ["PYRO10"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro followLoss="0.1" ')),
    ("pyro11-margin", ["PYRO11"], lambda _: PYRO_FOLLOW.replace('followMargin="2"', 'followMargin="4"')),
    ("pyc1-derived", ["PYC1"], lambda _: PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource crater="pit" start="1"/>')),
    ("pyc2-orphan", ["PYC2"], lambda _: PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource heatFraction="0.2"/>')),
    ("pyc3-time", ["PYC3"], lambda _: PYRO_CRATER.replace('<pyroImpulse crater="pit" heatFraction="0.2"/>', '<pyroImpulse density="1"/>')),
    ("pyc4-authored", ["PYC4"], lambda _: PYRO_CRATER.replace('<crater id="pit" source="rock" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("ocn8-coupling", ["OCN8"], lambda _: OCEAN.replace('<ocean ', '<ocean bodyCoupling="buoyancy" ')),
    ("ocn9-drag", ["OCN9"], lambda _: OCEAN.replace('<ocean ', '<ocean bodyDrag="1" ')),
    ("ocean-density-zero", ["S06"], lambda _: OCEAN.replace('<ocean ', '<ocean density="0" ')),
    ("crt10-mantle-without-source", ["CRT10"], lambda _: CRATER.replace('<crater ', '<crater mantle="true" ', 1)),
    ("crt10-bulking-without-source", ["CRT10", "CRT11"], lambda _: CRATER.replace('<crater ', '<crater bulking="1.1" ', 1)),
    ("crt11-bulking-without-mantle", ["CRT11"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" bulking="1.1"')),
    ("crt10-repose-without-source", ["CRT10"], lambda _: CRATER.replace('<crater ', '<crater repose="35" ', 1)),
    ("crt12-repose-with-mantle", ["CRT12"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true" repose="35"')),
    ("crt12-repose-range", ["S06"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" repose="75"')),
    ("crt11-bulking-range", ["S06"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true" bulking="1.5"')),
    ("crt8-self", ["CRT8"], lambda _: CRATER_IMPACT.replace('source="rock"', 'source="ground"')),
    ("v10", ["V10"], lambda _: POINTS.replace('version="1.2"', 'version="1.1"')),
    ("c17-points-count", ["C17"], lambda _: POINTS.replace('<repeat id="r"', '<repeat id="r" count="3"')),
    ("c74", ["C74"], lambda _: POINTS.replace('<shape ', '<points type="list" at="0,0"/><shape ')),
    ("c75", ["C75"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="along-path" path="M0,0 L10,0"')),
    ("c76", ["C76"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="scatter" count="3" width="10"')),
    ("c77", ["C77"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="vertices"')),
    ("c78", ["C78"], lambda _: POINTS.replace('<points type="grid" columns="3" rows="2"', '<points type="list"')),
    ("c79", ["C79"], lambda _: POINTS.replace('<animate property="spacingX">', '<animate property="rows">')),
    ("c80", ["C80"], lambda _: POINTS.replace('<repeat id="r"', '<repeat id="r" from="1"')),
    ("v11", ["V11"], lambda _: CONNECTOR.replace('version="1.2"', 'version="1.1"')),
    ("c60", ["C60"], lambda _: CONNECTOR.replace(' to="b"', '')),
    ("c61", ["C61"], lambda _: CONNECTOR.replace(' to="b"', ' to="b" toAnchor="top" toX="1" toY="2"')),
    ("c62", ["C62"], lambda _: CONNECTOR.replace(' to="b"', ' to="b" toX="1"')),
    ("c63", ["C63"], lambda _: CONNECTOR.replace('route="orthogonal"', 'route="curved" points="1,1"')),
    ("c64", ["C64"], lambda _: CONNECTOR.replace('<animate property="trimEnd">', '<animate property="rotation">')),
    ("r48-from", ["R48-from"], lambda _: CONNECTOR.replace('<shape id="a" shape="rect"', '<shape id="a" threeD="true" shape="rect"')),
    ("r48-to", ["R48-to"], lambda _: CONNECTOR.replace('<shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/>', '<repeat id="r" count="1"><shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/></repeat>')),
    ("r49", ["R49"], lambda _: CONNECTOR.replace('</composition>', '<shape id="p" shape="rect" width="2" height="2" parent="c"/></composition>')),
    ("r50", ["R50"], lambda _: CONNECTOR.replace('label="note"', 'label="a"')),
    ("v9", ["V9"], lambda _: PDF.replace('version="1.2"', 'version="1.1"')),
    ("c66", ["C66"], lambda _: PDF.replace(f' sha256="{"0" * 64}"', '')),
    ("c67", ["C67", "R52"], lambda _: PDF.replace(' regionLayer="page"', '')),
    ("c68", ["C68"], lambda _: PDF.replace(' regionLayer="page"', ' regionLayer="page" parent="page"')),
    ("c69", ["C69"], lambda _: PDF.replace('</composition>', '<shape id="plain" shape="rect" width="4"/></composition>')),
    ("r51", ["R51", "R52"], lambda _: PDF.replace('region="w"', 'region="page"')),
    ("r52", ["R52"], lambda _: PDF.replace('<layer id="page" asset="paper" x="8" y="8"/>', '<layer id="page" asset="paper" x="8" y="8"/><layer id="other" asset="paper"/>').replace('regionLayer="page"', 'regionLayer="mark"')),
    ("v5-joint", ["V5"], lambda _: JOINT.replace('version="1.2"', 'version="1.1"')),
    ("v5-morph", ["V5"], lambda _: MORPH.replace('version="1.2"', 'version="1.1"')),
    ("pen1", ["PEN1"], lambda _: STROKE_TEXT.replace(' strokeFont="hand"', '')),
    # SREP 21: fontPolicy="pinned" (C70 to C73)
    ("c70-font-file", ["C70"], lambda _: PINNED_FONTS.replace(' fallback="Inter"', ' fallback="Inter" fontFile="../media/inter.ttf"')),
    ("c71-unpinned-asset", ["C71"], lambda _: re.sub(r' sha256="[0-9a-f]+"', '', PINNED_FONTS)),
    ("c72-host-family", ["C72"], lambda _: PINNED_FONTS.replace('font="Inter"', 'font="Helvetica"')),
    ("c73-host-fallback", ["C73"], lambda _: PINNED_FONTS.replace('fallback="Inter"', 'fallback="Inter, Arial"')),
    ("pen2", ["PEN2"], lambda _: STROKE_TEXT.replace('<strokeFont id="hand"', '<mesh id="hand"').replace('src="../media/test.jhf"', 'src="../media/robot.glb"')),
    ("v5-stroke-text", ["V5"], lambda _: STROKE_TEXT.replace('version="1.2"', 'version="1.1"')),
    ("mov1", ["MOV1"], lambda _: MODEL_SELECT.replace('materialOverride="stone:clay old:clay"', 'materialOverride="stone:clay :clay"')),
    ("mov2", ["MOV2"], lambda _: MODEL_SELECT.replace('materialOverride="stone:clay old:clay"', 'materialOverride="stone:clay old:nosuch"')),
    ("txt2", ["TXT2"], lambda _: TEXT3D.replace('primitive="text" text="HI"', 'primitive="box"')),
    ("solid-colliders-tracking", ["P3D6", "PYRO8"], lambda _: SOLID_COLLIDERS.replace('<object3D id="letters" primitive="text" text="O"/>', '<object3D id="letters" primitive="text" text="O"><animate property="tracking"><key time="0" value="10"/></animate></object3D>')),
    ("msq1", ["MSQ1"], lambda _: MESH_SEQUENCE.replace('version="1.3"', 'version="1.2"')),
    ("msq2", ["MSQ2", "A04"], lambda _: MESH_SEQUENCE.replace('last="1"', 'last="-1"')),
    ("msq3", ["MSQ3"], lambda _: MESH_SEQUENCE.replace('first="0"', 'sha256="' + '0' * 64 + '" first="0"')),
    ("msq4", ["MSQ4"], lambda _: MESH_SEQUENCE.replace('y="32"/>', 'y="32"><rigidBody type="static"/></object3D>')),
    ("geo1", ["GEO1"], lambda _: GLOBE.replace('version="1.3"', 'version="1.2"')),
    ("geo2", ["GEO2"], lambda _: GLOBE.replace('terrainTileSize="2"', 'terrainTileSize=" +260 "')),
    ("geo3", ["GEO3"], lambda _: GLOBE.replace('segments="32"/>', 'segments="32"><animate property="planetRadius"><key time="0" value="1000"/></animate></object3D>')),
    ("ocn1", ["OCN1"], lambda _: OCEAN.replace('version="1.3"', 'version="1.2"')),
    ("ocn2", ["OCN2"], lambda _: OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" bathymetryEncoding="terrarium"')),
    ("ocn3", ["OCN3"], lambda _: OCEAN.replace('width="8"', 'width="8.1"')),
    ("ocn4", ["OCN4"], lambda _: OCEAN.replace('</ocean>', '<animate property="bottomDepth"><key time="0" value="2"/></animate></ocean>')),
    ("ocn5", ["OCN5"], lambda _: OCEAN.replace('</ocean>', '<whitewater start="1" end="0.5"/></ocean>')),
    ("pyc5-closed", ["PYC5"], lambda _: PYRO_BLAST.replace('boundary="open" ', '')),
    ("pyc6-outside", ["PYC6"], lambda _: PYRO_BLAST.replace('x="1"', 'x="5"')),
    ("pyro-blast-gamma", ["S06"], lambda _: PYRO_BLAST.replace('<pyroBlast ', '<pyroBlast gamma="4" ')),
    ("pyro-advection", ["S06"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro advection="rk4" ')),
    ("pyro-solver", ["S06"], lambda _: VALID["pyro"].replace('<pyro ', '<pyro solver="cg" ')),
    ("ocn6", ["OCN6"], lambda _: OCEAN_COUPLED.replace('colliders="seabed rock"', 'colliders="seabed rock seabed"')),
    ("ocn7", ["OCN7"], lambda _: OCEAN_COUPLED.replace('<object3D id="rock" primitive="sphere" radius="1" y="-3"/>', '<object3D id="rock" primitive="sphere" radius="1" y="-3"><animate property="radius"><key time="0" value="1"/><key time="1" value="2"/></animate></object3D>')),
    ("whitewater-checkpoint-range", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater checkpointMemoryMiB="4097" ')),
    ("whitewater-foam-mode", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater foamMode="mix" ')),
    ("ocn14-foam-radius", ["OCN14"], lambda _: VALID["ocean"].replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('<whitewater ', '<whitewater foamMode="albedo" foamRadius="40" ')),
    ("whitewater-foam-albedo-range", ["S06"], lambda _: VALID["ocean"].replace('<whitewater ', '<whitewater foamAlbedo="1.5" ')),
    ("ocean-order", ["S06"], lambda _: OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" order="3"')),
    ("p3d1", ["P3D1"], lambda _: PARTICLES3D.replace('version="1.3"', 'version="1.2"')),
    ("p3d2", ["P3D2"], lambda _: PARTICLES3D.replace('rate="0"', 'rate="0" emitterShape="mesh"')),
    ("p3d3", ["P3D3"], lambda _: PARTICLES3D.replace('lifetime="2"', 'lifetime="2" lifetimeVariance="2"')),
    ("p3d7-derived", ["P3D7"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst crater="pit" time="1" count="200"/>')),
    ("p3d7-time", ["P3D3", "P3D7"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst count="200"/>')),
    ("p3d8-authored", ["P3D8"], lambda _: EJECTA_CRATER.replace('<crater id="pit" source="rock" targetMaterial="softRock"/>', '<crater id="pit" radius="4" rimWidth="1"/>')),
    ("p3d9-orphan", ["P3D9"], lambda _: EJECTA_CRATER.replace('<burst crater="pit" count="200"/>', '<burst time="0" count="200" angle="30"/>')),
    ("ocn13-splash", ["OCN13"], lambda _: EJECTA_CRATER.replace('</composition>', '<ocean id="sea" width="16" depth="16" cellSize="1" splash="debris pit"/></composition>')),
    ("p3d11-gas", ["P3D11"], lambda _: PARTICLES3D.replace('<particles3D id="dust"', '<particles3D id="dust" gas="floor"')),
    ("p3d10-angle", ["P3D10"], lambda _: EJECTA_CRATER.replace('count="200"/>', 'count="200" angle="80"/>')),
    ("bedresponse-value", ["S06"], lambda _: OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="linear" ')),
    ("ocn10-derived", ["OCN10"], lambda _: OCEAN_ENTRY.replace('<waterImpulse source="rock"/>', '<waterImpulse source="rock" radius="2"/>')),
    ("ocn11-unlisted", ["OCN11"], lambda _: OCEAN_ENTRY.replace(' colliders="rock"', '')),
    ("ocn11-static", ["OCN11"], lambda _: OCEAN_ENTRY.replace('<rigidBody mass="5"/>', '<rigidBody type="static"/>')),
    ("ocn12-twice", ["OCN12"], lambda _: OCEAN_ENTRY.replace('<waterImpulse source="rock"/>', '<waterImpulse source="rock"/><waterImpulse source="rock"/>')),
    ("p3d4", ["P3D4"], lambda _: PARTICLES3D.replace('</particles3D>', '<animate property="lifetime"><key time="0" value="2"/></animate></particles3D>')),
    ("p3d5", ["P3D5"], lambda _: PARTICLES3D.replace('colliders="floor"', 'colliders="dust"')),
    ("p3d6", ["P3D6"], lambda _: PARTICLES3D.replace('depth="64"/>', 'depth="64"><animate property="width"><key time="0" value="64"/><key time="1" value="32"/></animate></object3D>')),
    ("pyro1", ["PYRO1"], lambda _: PYRO.replace('voxelSize="1"', 'voxelSize="3"')),
    ("pyro2", ["PYRO2"], lambda _: PYRO.replace('end="0.35"', 'end="0.1"')),
    ("pyro3", ["PYRO3"], lambda _: PYRO.replace('shape="sphere" radius="2"', 'shape="box"')),
    ("pyro4", ["PYRO4"], lambda _: PYRO.replace('radius="2"', 'radius="2" scaleX="0"')),
    ("pyro5", ["PYRO5"], lambda _: PYRO.replace('shape="sphere" radius="2"', 'shape="mesh"')),
    ("pyro6", ["PYRO6"], lambda _: PYRO_MESH.replace('end="0.35"/>', 'end="0.35"><animate property="mesh"><key time="0" value="source-mesh"/></animate></pyroSource>')),
    ("pyro7", ["PYRO7"], lambda _: PYRO_COLLIDERS.replace('colliders="solid"', 'colliders="cloud"')),
    ("pyro8", ["PYRO8"], lambda _: PYRO_COLLIDERS.replace('depth="2"/>', 'depth="2"><animate property="width"><key time="0" value="2"/><key time="1" value="4"/></animate></object3D>')),
    ("v8-volumes", ["V8"], lambda _: VOLUME.replace('version="1.3"', 'version="1.2"')),
    ("vol1", ["VOL1"], lambda _: VOLUME.replace(' volume="smoke"', '')),
    ("vol2", ["VOL2"], lambda _: VOLUME.replace('volume="smoke"', 'volume="cloud"')),
    ("vol3", ["VOL3"], lambda _: VOLUME.replace('primitive="volume"', 'primitive="box"')),
    ("vol4", ["VOL4"], lambda _: VOLUME.replace(' boundsMaxZ="2"', '')),
    ("vol5", ["VOL5"], lambda _: VOLUME.replace('<medium ', '<medium blackbody="true" ')),
    ("vol6", ["VOL6"], lambda _: VOLUME_SEQUENCE.replace(' last="0"', '')),
    ("vol9", ["VOL9"], lambda _: VOLUME_SEQUENCE.replace('interpolation="linear"', 'interpolation="advect" velocityGridX="velocity.x" velocityGridY="velocity.y"')),
    ("vol8", ["VOL8"], lambda _: VOLUME_BAKED.replace('format="srvseq"', 'format="srvseq" fps="1"')),
    ("vol-scatter-bounces-range", ["S06"], lambda _: VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#808080" scatterBounces="33" ')),
    ("vol-scatter-bounces-fraction", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium scatterBounces="1.5" ')),
    ("vol10", ["VOL10"], lambda _: VOLUME.replace('<medium ', '<medium lightGridCell="2" ')),
    ("med1", ["MED1"], lambda _: VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phaseAlpha="2" ')),
    ("med2", ["MED2"], lambda _: VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phase="draine" dropletDiameter="20" ')),
    ("med3", ["MED3"], lambda _: VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phase="rayleigh" anisotropy="0.3" ')),
    ("med4", ["MED4"], lambda _: VOLUME.replace('version="1.3"', 'version="1.7"').replace('<medium ', '<medium phase="rayleigh" ')),
    ("med4-after-v8", ["V8"], lambda _: VOLUME.replace('version="1.3"', 'version="1.2"').replace('<medium ', '<medium phase="rayleigh" ')),
    ("med-phase-enum", ["S06"], lambda _: VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phase="mie" ')),
    ("med-droplet-range", ["S06"], lambda _: VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phase="fog" dropletDiameter="60" ')),
    ("vol-lighting-enum", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="fast" ')),
    ("vol-light-grid-cell", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="grid" lightGridCell="0" ')),
    ("vol-light-grid-directions", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="grid" lightGridDomeDirections="4" ')),
    ("vol-light-grid-memory", ["S06"], lambda _: VOLUME.replace('<medium ', '<medium lighting="grid" lightGridMemoryMiB="0" ')),
    ("vol7", ["VOL7"], lambda _: VOLUME_SEQUENCE.replace('id="smoke"', 'id="smoke" sha256="' + '0' * 64 + '"')),
    # ---- structure
    ("s01-root", ["S01"], lambda t: MINIMAL.replace("scene", "movie")),
    ("s02-unknown-element", ["S02"], ins_comp('<lyer id="z" asset="logo"/>')),
    ("s02-out-of-order", ["S02"], sub("  <composition>", "  <project width=\"1\" height=\"1\" fps=\"1\" duration=\"1\"/>\n  <composition>")),
    ("s03-missing-child", ["S03"], lambda t: '<scene version="1.1"><project width="1" height="1" fps="1" duration="1"/></scene>\n'),
    ("s03-empty-animate", ["S03"], ins_comp('<layer id="z" asset="logo"><animate property="x"/></layer>')),
    ("s04-unknown-attribute", ["S04"], sub('<bus id="fx"/>', '<bus id="fx" gian="3"/>')),
    ("s05-missing-attribute", ["S05"], sub('<light id="key" type="directional"/>', '<light id="key"/>')),
    ("s06-enum", ["S06"], sub('codec="h264"', 'codec="h266"')),
    ("s06-bound", ["S06"], sub('<layer id="shot-a" asset="logo" end="4"/>', '<layer id="shot-a" asset="logo" end="4" opacity="1.5"/>')),
    ("s06-pattern-fps", ["S06"], sub('fps="12"', 'fps="12/0"')),
    ("s06-colour", ["S06"], sub('<stop offset="1" color="#220044"/>', '<stop offset="1" color="1.2,0,0"/>')),
    ("s06-length", ["S06"], sub('group id="hero" x="50%"', 'group id="hero" x="50px"')),
    ("s06-double", ["S06"], sub('<marker id="drop" time="4.2"', '<marker id="drop" time="4,2"')),
    ("s06-integer", ["S06"], sub('seed="7"', 'seed="-7"')),
    ("s06-idrefs-empty", ["S06", "R53"], sub('<adjustment id="grade-all" effects="grade"/>', '<adjustment id="grade-all" effects=" "/>')),
    ("s07-text", ["S07"], sub("<master/>", "<master>loud</master>")),
    ("s09-duplicate-id", ["S09"], sub('<bus id="fx"/>', '<bus id="fx"/>\n    <bus id="fx"/>')),
    ("s10-dangling-idref", ["S10", "R54"], sub('<audiogram id="wave" source="music-track"', '<audiogram id="wave" source="musik"')),
    # ---- version check
    ("v1-sections", ["V1"], lambda t: v10("", '<markers><marker time="1"/></markers>')),
    ("v2-outputs", ["V2"], lambda t: '<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/><output path="a.mp4" codec="h264"/><output path="b.mp4" codec="h264"/><composition/></scene>\n'),
    ("v3-elements", ["V3"], lambda t: v10('<sequence id="s"/>')),
    ("v5-elements", ["V5"], sub('<scene version="1.2">', '<scene version="1.1">')),
    ("v4-assets", ["V4"], lambda t: '<scene version="1.0"><project width="640" height="360" fps="25" duration="2"/><assets><formula id="f" tex="x" width="10" height="10"/></assets><composition/></scene>\n'),
    # ---- co-occurrence
    ("c1", ["C1"], sub('<vector id="icon" shape="path" path="M0 0 L10 0 L5 10 Z"', '<vector id="icon" shape="path"')),
    ("c2", ["C2"], sub('<vector id="icon" shape="path" path="M0 0 L10 0 L5 10 Z"', '<vector id="icon" shape="svg"')),
    ("c3", ["C3"], sub('<shape id="dot" shape="ellipse"', '<shape id="dot" shape="path"')),
    ("c65", ["C65"], sub('<shape id="dot" shape="ellipse"', '<shape id="dot" shape="ellipse" markerEnd="arrow"')),
    ("c4", ["C4"], sub('<mask type="ellipse" width="800" height="800"/>', '<mask type="path"/>')),
    ("c5", ["C5"], sub('<mask type="ellipse" width="800" height="800"/>', '<mask type="ellipse" width="800"/>')),
    ("c6", ["C6"], sub('primitive="mesh" mesh="robot"', 'primitive="mesh"')),
    ("c7", ["C7"], sub('primitive="mesh" mesh="robot"', 'primitive="text"')),
    ("c8", ["C8"], sub('primitive="mesh" mesh="robot"', 'primitive="extrude"')),
    ("c9", ["C9"], sub('<constraint id="pin1" type="pin" a="sparks"', '<constraint id="pin1" type="pin" a="sparks" b="bot"')),
    ("c10", ["C10"], sub('a="sparks" x="0" y="0"', 'a="sparks" x="0"')),
    ("c11", ["C11"], sub('<constraint id="pin1" type="pin" a="sparks" x="0" y="0"/>', '<constraint id="pin1" type="spring" a="sparks"/>')),
    ("c12", ["C12"], sub('<text id="rich" width="800" height="100" size="40">', '<text id="rich" width="800" height="100" size="40" text="both">')),
    ("c13", ["C13"], sub('minSize="24" maxSize="96"', 'minSize="120" maxSize="96"')),
    ("c13-exponent", ["C13"], sub('minSize="24" maxSize="96"', 'minSize="1.2e2" maxSize="96"')),
    ("c14", ["C14"], sub('letterSpacing="2"', 'letterSpacing="400"')),
    ("c15", ["C15"], sub('<layer id="spinner" asset="spin">', '<layer id="spinner" asset="spin" speed="2">')),
    ("c15-reverse-1", ["C15"], sub('<layer id="spinner" asset="spin">', '<layer id="spinner" asset="spin" reverse="1">')),
    ("c16", ["C16"], sub('fit="contain" boxWidth="400" boxHeight="300"', 'fit="contain" boxWidth="400"')),
    ("c17", ["C17"], sub('<repeat id="grid" count="3">', '<repeat id="grid">')),
    ("c18", ["C18"], sub('<repeat id="list" over="rows">', '<repeat id="list" over="headline">')),
    ("c19", ["C19"], sub('<effect id="grade" type="lut" src="../media/grade.cube"/>', '<effect id="grade" type="lut"/>')),
    ("c20", ["C20"], sub('<transition id="xf" type="crossfade" from="shot-a" to="shot-b"/>', '<transition id="xf" type="crossfade"/>')),
    ("c21", ["C21"], sub('type="crossfade" from="shot-a"', 'type="shader" from="shot-a"')),
    ("c22", ["C22"], sub('type="crossfade" from="shot-a"', 'type="luma" from="shot-a"')),
    ("c23", ["C23"], sub('from="shot-a" to="shot-b"', 'from="hero" to="shot-b"')),
    ("c24", ["C24"], sub('from="shot-a" to="shot-b"', 'from="shot-a" to="bot"')),
    ("c25", ["C25"], ins_comp('<layer id="z" asset="logo"><deform><modifier type="skin"/></deform></layer>')),
    ("c26", ["C26"], ins_comp('<layer id="z" asset="logo"><deform><modifier type="corner-pin" corners="0 0 1 0 1 1 0"/></deform></layer>')),
    ("c27", ["C27"], ins_comp('<layer id="z" asset="logo"><transformConstraint type="follow-path"/></layer>')),
    ("c28", ["C28"], ins_comp('<layer id="z" asset="logo"><transformConstraint type="look-at"/></layer>')),
    ("c29", ["C29"], sub('<transformConstraint type="track" target="face"/>', '<transformConstraint type="track" target="hero"/>')),
    ("c30", ["C30"], sub('preset="custom" top="0.05"', 'preset="custom"')),
    ("c31", ["C31"], sub('<captionTrack id="en" language="en-US" safeArea="broadcast">', '<captionTrack id="en" language="en-US" safeArea="broadcast" src="../media/transcript.vtt">')),
    ("c32", ["C32"], lambda t: re.sub(r'(transcribe="vo-track" cache="\.\./media/transcript\.vtt") cacheSha256="[0-9a-f]+"', r'\1', t)),
    ("c33", ["C33"], sub('transcribe="vo-track"', 'transcribe="music"')),
    ("c34", ["C34"], sub("<master/>", "<master/>\n    <master/>")),
    ("c35", ["C35"], lambda t: MINIMAL.replace("<composition/>", '<composition/><audioMix><bus id="b"/></audioMix>')),
    ("c36", ["C36"], sub('<stop offset="0" color="#ffffff"/>\n      <stop offset="1" color="#ffffff00"/>', '<stop offset="0" color="#ffffff"/>')),
    ("c37", ["C37"], sub('<stop offset="0.5" color="var(--brand)"/>', '<stop offset="0.5" color="var(--brand)"/>\n      <stop offset="0.2" color="#000000"/>')),
    ("c38", ["C38"], sub('<key time="1" value="0.4"/>', '<key time="0.5" value="0.4"/>\n          <key time="0.25" value="0.5"/>')),
    ("c39", ["C39"], sub('interpolation="steps" steps="4"', 'interpolation="steps"')),
    ("c40", ["C40"], sub('interpolation="cubic-bezier" bezier="0.25,0.1,0.25,1"', 'interpolation="cubic-bezier"')),
    ("c41", ["C41"], sub('kind="speech" provider="acme-tts" model="v2" prompt="Welcome"', 'kind="music" provider="acme-tts" model="v2"')),
    ("c42", ["C42"], sub('codec="h264" layout="square"', 'codec="h264" proresProfile="hq" layout="square"')),
    ("c43", ["C43"], sub('codec="h264" layout="square"', 'codec="h264" alpha="true" layout="square"')),
    ("c44", ["C44"], sub('start="0" end="12"', 'start="5" end="4"')),
    # ---- references
    ("r1", ["R1"], sub('<layer id="shot-a" asset="logo"', '<layer id="shot-a" asset="rows"')),
    ("r2", ["R2"], sub('<layer id="title-layer" asset="title"', '<layer id="title-layer" asset="logo"')),
    ("r3", ["R3"], sub('audioBus="fx"', 'audioBus="music-track"')),
    ("r4", ["R4"], sub('material="chrome"', 'material="logo"')),
    ("r5", ["R5"], sub('mesh="robot" material', 'mesh="logo" material')),
    ("r6", ["R6"], sub('effects="glow-fx grade"', 'effects="glow-fx key"')),
    ("r6-duplicate", ["R6"], sub('effects="glow-fx grade"', 'effects="grade grade"')),
    ("r7", ["R7"], sub('lights="key dome"', 'lights="key sparks"')),
    ("c38-exponent", ["C38"], sub('<key time="1" value="0.4"/>', '<key time="1" value="0.4"/>\n          <key time="5e-1" value="0.5"/>')),
    ("r8", ["R8"], sub('matte="bg"', 'matte="logo"')),
    ("r9", ["R9"], sub('matte="bg"', 'matte="title-layer"')),
    ("r10", ["R10"], sub('matte="bg" parent="hero"', 'matte="bg" parent="title-layer"')),
    ("r11", ["R11"], sub('symbol="card"', 'symbol="hero"')),
    ("r12", ["R12"], sub('layout="square"', 'layout="broadcast"')),
    ("r13", ["R13"], sub('variant="dark"', 'variant="rows"')),
    ("r14", ["R14"], sub('burnCaptions="en"', 'burnCaptions="vo-track"')),
    ("r15", ["R15"], sub('<audioTrack id="clip-audio" asset="clip"/>', '<audioTrack id="clip-audio" asset="logo"/>')),
    ("r15-video-without-audio", ["R15"], sub('hasAudio="true"', 'hasAudio="false"')),
    ("r16", ["R16"], sub('bus="fx" duckUnder', 'bus="vo-track" duckUnder')),
    ("r17", ["R17"], sub('duckUnder="vo-track"', 'duckUnder="vo-track music-track"')),
    ("r18", ["R18"], sub('<bind param="headline"', '<bind param="rows"')),
    ("r19", ["R19"], sub('<layout id="square" width="1080" height="1080" safeArea="broadcast">', '<layout id="square" width="1080" height="1080" safeArea="en">')),
    ("r20", ["R20"], sub('focusTarget="hero" target="hero"', 'focusTarget="hero" target="card-bg"')),
    ("r21", ["R21"], sub('startMarker="drop"', 'startMarker="hero"')),
    ("r21-key", ["R21"], sub('marker="drop"', 'marker="hero"')),
    ("r22", ["R22"], sub('basedOn="heading"', 'basedOn="title"')),
    ("r22-style", ["R22"], sub('size="72" style="heading"', 'size="72" style="logo"')),
    ("r23", ["R23"], sub('fontAsset="inter"', 'fontAsset="logo"')),
    ("r36", ["R36"], sub('<geoLayer geo="places"', '<geoLayer geo="logo"')),
    ("r37", ["R37"], sub('<route geo="places"', '<route geo="logo"')),
    ("r26", ["R26"], sub('fit="places"', 'fit="places logo"')),
    ("c45", ["C45"], sub('<route points="0,0 20,20"', '<route')),
    # ---- output segments
    ("c54", ["C54"], sub('<output id="short" start="0" path', '<output id="short" start="1" path')),
    ("c55", ["C55"], sub('<segment fromMarker="drop" to="8"', '<segment fromMarker="drop"')),
    ("c56", ["C56"], sub('<segment from="0" to="4"', '<segment from="4" to="2"')),
    ("c57", ["C57"], sub('<segment fromMarker="drop" to="8"', '<segment from="3" fromMarker="drop" to="8"')),
    ("c58", ["C58"], sub('<transition type="crossfade" duration="0.3" alignment="end"/>', '<transition type="crossfade" duration="0.3" alignment="end"/>\n      <transition type="cut"/>')),
    ("c59", ["C59"], sub('<transition type="crossfade" duration="0.3" alignment="end"/>', '<transition type="morph" duration="0.3"/>')),
    ("r38", ["R38"], sub('fromMarker="drop"', 'fromMarker="hero"')),
    ("r39", ["R39"], sub('audioTracks="music-track"', 'audioTracks="music-track fx"')),
    ("r39-bus", ["R39"], sub('audioBuses="fx"', 'audioBuses="fx music-track"')),
    ("c59-luma", ["C59"], sub('<transition type="crossfade" duration="0.3" alignment="end"/>', '<transition type="luma" matte="card" duration="0.3"/>')),
    ("r40", ["R40"], sub('overlay="card"', 'overlay="logo"')),
    ("r41", ["R41"], sub('transcribe="short-vo"', 'transcribe="vo-track"')),
    # ---- schema 1.1.3
    ("v6-elements", ["V6"], lambda t: v10('<particleEmitter id="p"><burst time="1" count="2"/></particleEmitter>')),
    ("v7-primitives", ["V7"], lambda t: v10('<object3D id="o" primitive="torus"/>')),
    ("r10-any-node", ["R10"], ins_comp('<flock id="fk" width="10" height="10" parent="fk"/>')),
    ("r30-outline", ["R30-outline"], sub('outline="#FFFFFF"', 'outline="url(#nope)"')),
    ("r31-baseColor", ["R31-baseColor"], sub('<material id="chrome" metallic="1"', '<material id="chrome" baseColor="var(--nope)" metallic="1"')),
    ("c50", ["C50"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" shape="sprite">')),
    ("r32", ["R32"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" sprite="music">')),
    ("r42", ["R42"], sub('<pattern id="checker" asset="logo"/>', '<pattern id="checker" asset="music"/>')),
    ("r43", ["R43"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" emitterAsset="music">')),
    ("r44", ["R44"], sub('<effect id="grade" type="lut" src="../media/grade.cube"/>', '<effect id="grade" type="lut" src="../media/grade.cube"/>\n    <effect id="dm" type="displacement-map" source="logo"/>')),
    ("r47", ["R47"], sub('<generator id="noise" kind="fractal-noise" width="512" height="512">', '<generator id="noise" kind="fractal-noise" width="512" height="512" lineWidth="2">')),
    ("r33", ["R33"], ins_comp('<erosion id="er" width="10" height="10" heightmap="music"/>')),
    ("r34", ["R34"], sub('<particleEmitter id="sparks" color="#ffcc00">', '<particleEmitter id="sparks" color="#ffcc00" forceFields="gravity music">')),
    ("r35", ["R35"], sub('<pin lon="5" lat="5" label="Here"/>', '<pin lon="5" lat="5" label="Here" textStyle="logo"/>')),
    ("c51", ["C51"], ins_comp('<object3D id="cl" primitive="clay"/>')),
    ("c52", ["C52"], ins_comp('<fluid id="fl" width="10" height="10"><fluidSource start="2" end="1"/></fluid>')),
    ("c53", ["C53"], sub('palette="#F7FBFF #08306B"', 'palette="#F7FBFF #08306B" domain="3 1"')),
    ("s06-latitude", ["S06"], sub('<pin lon="5" lat="5"', '<pin lon="5" lat="95"')),
    ("r27", ["R27"], sub('<basemap tiles="streets"', '<basemap tiles="places"')),
    ("c47", ["C47"], sub('<object3D id="earth" primitive="globe" map="atlas"', '<object3D id="earth" primitive="globe"')),
    ("r28", ["R28"], sub('primitive="globe" map="atlas"', 'primitive="globe" map="streets"')),
    ("r29", ["R29"], sub('terrain="streets"', 'terrain="atlas"')),
    ("c48", ["C48"], sub('<rigidBody type="static" shape="trimesh"/>', '<rigidBody shape="trimesh"/>')),
    ("c46", ["C46"], sub('<tiles id="streets" src="../media/streets.pmtiles"', '<tiles id="streets"')),
]
for attr, anchor, repl in [
    ("fill", 'shape="rounded-rect" width="400" height="300" fill="var(--bg)"', 'shape="rounded-rect" width="400" height="300" fill="url(#logo)"'),
    ("stroke", 'stroke="url(#glow)"', 'stroke="url(#nope)"'),
    ("color", '<span color="url(#sunset)"', '<span color="url(#sun)"'),
    ("background", '<symbol id="card" width="400" height="300">', '<symbol id="card" width="400" height="300" background="url(#x)">'),
    ("paint", '<effect id="glow-fx" type="glow"', '<effect id="glow-fx" type="glow" paint="url(#nope)"'),
    ("activeColor", '<captionTrack id="en" language="en-US"', '<captionTrack id="en" language="en-US" activeColor="url(#nope)"'),
    ("highlight", '<text id="title" text', '<text id="title" highlight="url(#nope)" text'),
    ("strokeColor", '<text id="title" text', '<text id="title" strokeColor="url(#nope)" text'),
]:
    CASES.append((f"r24-{attr}", [f"R24-{attr}"], sub(anchor, repl)))
for attr, anchor, repl in [
    ("fill", 'fill="var(--bg)"', 'fill="var(--missing)"'),
    ("stroke", 'stroke="var(--brand)"', 'stroke="var(--brnd)"'),
    ("color", 'color="var(--brand)" weight', 'color="var(--none)" weight'),
    ("background", 'background="var(--bg)"', 'background="var(--bgg)"'),
    ("paint", '<generator id="noise" kind="fractal-noise"', '<generator id="noise" paint="var(--nope)" kind="fractal-noise"'),
    ("activeColor", '<captionTrack id="en" language="en-US"', '<captionTrack id="en" language="en-US" activeColor="var(--nope)"'),
    ("highlight", '<text id="title" text', '<text id="title" highlight="var(--nope)" text'),
    ("strokeColor", '<span color="#ffffff">', '<span color="#ffffff" strokeColor="var(--nope)">'),
]:
    CASES.append((f"r25-{attr}", [f"R25-{attr}"], sub(anchor, repl)))

# the paint and colour attributes 1.1.3 added to the url(#) and var(--) checks: one insertion point each
EXTRA_REF_SITES = {
    "paint2": '<generator id="noise" kind="fractal-noise"',
    "foreground": '<code id="qr" kind="qr"',
    "noData": '<geoLayer geo="places"',
    "headFill": '<route points="0,0 20,20"',
    "attenuationColor": '<material id="chrome"',
    "baseColor": '<material id="chrome"',
    "emissive": '<material id="chrome"',
    "sheenColor": '<material id="chrome"',
    "specularColor": '<material id="chrome"',
    "colorEnd": '<particleEmitter id="sparks"',
    "keyColor": '<effect id="glow-fx"',
    "shadowColor": '<span color="#ffffff"',
}

def with_attr(attr, value):
    if attr in ("colorHigh", "colorLow"):
        return ins_comp(f'<erosion id="er" width="10" height="10" {attr}="{value}"/>')
    if attr == "outline":  # the map already has an outline: replace it
        return sub('outline="#FFFFFF"', f'outline="{value}"')
    site = EXTRA_REF_SITES[attr]
    return sub(site, f'{site} {attr}="{value}"')

for attr in ["colorEnd", "colorHigh", "colorLow", "headFill", "noData", "outline", "paint2"]:
    if attr != "outline":
        CASES.append((f"r30-{attr}", [f"R30-{attr}"], with_attr(attr, "url(#nope)")))
for attr in ["attenuationColor", "baseColor", "colorEnd", "colorHigh", "colorLow", "emissive", "foreground", "headFill",
             "keyColor", "noData", "outline", "paint2", "shadowColor", "sheenColor", "specularColor"]:
    if attr != "baseColor":
        CASES.append((f"r31-{attr}", [f"R31-{attr}"], with_attr(attr, "var(--nope)")))

# Asset cases: the oracle sees a valid document; the Rust verifier must not.
# SREPs 66 and 69: the version gate V12, PRG1 to PRG3 and STP1 on programs
CASES += [
    ("v12-program", ["V12"], lambda _: PROGRAMS.replace('version="1.6"', 'version="1.5"')),
    ("prg1-param-twice", ["PRG1"], lambda _: PROGRAMS.replace('<param name="side" value="1"/>', '<param name="side" value="1"/><param name="side" value="0"/>')),
    ("prg2-over-node-program", ["C18", "PRG2"], lambda _: PROGRAMS.replace('over="rows"', 'over="build"')),
    ("prg3-step-size", ["PRG3"], lambda _: PROGRAMS.replace(' width="8" height="8"', '')),
    ("stp1-build-program", ["STP1"], lambda _: PROGRAMS.replace(' seed="3"', ' seed="3" prewarm="2"')),
]
ASSET_CASES = [
    ("a01-missing-file", ["A01"], sub('src="../media/clip.mp4"', 'src="../media/clip-missing.mp4"')),
    ("a02-hash-mismatch", ["A02"], lambda t: re.sub(r'(src="\.\./media/music\.wav" sha256=")[0-9a-f]{4}', r'\g<1>0000', t)),
    ("a02-generated-cache", ["A02"], lambda t: re.sub(r'(cache="\.\./media/voice\.wav" cacheSha256=")[0-9a-f]{4}', r'\g<1>ffff', t)),
    ("a04-sequence-frames", ["A04"], sub('first="1" last="5"', 'first="1" last="9"')),
    ("prg10-program-hash", ["PRG10"], lambda _: PROGRAMS.replace(PROGRAM_SHA["rect"], "0" * 64)),
    ("prg10-program-missing", ["PRG10"], lambda _: PROGRAMS.replace("../media/rect.wasm", "../media/missing.wasm")),
    # the time-frames draft: TF1, and the clauses of TF4 and TF5 that read the ephemeris file's bodies
    ("tf1-digest", ["TF1"], lambda _: re.sub(r'(digest="sha256:)[0-9a-f]', lambda m: m.group(1) + ('1' if TF.split('digest="sha256:')[1][0] == '0' else '0'), TF)),
    ("tf1-not-spk", ["TF1"], lambda _: TF.replace("../media/de440s-excerpt.bsp", "../media/music.wav")),
    ("tf4-body-not-in-file", ["TF4"], lambda _: TF.replace('body="301"', 'body="499"')),
    ("tf5-origin-not-in-file", ["TF5"], lambda _: TF.replace('origin="399"', 'origin="499"')),
    # the star-fields draft: SF1 reads the catalogue file
    ("sf1-digest", ["SF1"], lambda _: re.sub(r'(digest="sha256:)[0-9a-f]', lambda m: m.group(1) + ('1' if SF.split('digest="sha256:')[1][0] == '0' else '0'), SF)),
    ("sf1-not-a-catalogue", ["SF1"], lambda _: SF.replace("../media/bsc5-compact.csv.gz", "../media/music.wav")),
]
# Errors of the engine's own rules (rules.rs), outside XSD and Schematron: the oracle accepts these documents.
RULE_CASES = [
    # an 821×442 H.264 output in yuv420p: libx264 refuses an odd 4:2:0 frame when the encode starts
    ("out1-odd-h264-frame", ["OUT1"], lambda _: MINIMAL.replace('width="1920" height="1080" fps="30" duration="5"/>', 'width="821" height="442" fps="30" duration="1"/><output id="web" path="out/web.mp4" codec="h264"/>')),
]
WARN_CASES = [
    ("a03-remote", ["A03"], sub('src="../media/clip.mp4"', 'src="https://cdn.example.com/clip.mp4"')),
    ("a04-sequence-hold", ["A04"], sub('first="1" last="5"', 'first="1" last="9" missingFrame="hold"')),
    ("w02-open-face", ["W02"], lambda _: PYRO.replace('width="8" height="8" depth="8" voxelSize="1"', 'width="64" height="64" depth="64" voxelSize="1" boundary="open"').replace('<pyroSource shape="sphere" radius="2"', '<pyroSource shape="sphere" y="-28" radius="2"')),
    ("w12-denoise", ["W12"], lambda _: BLACKHOLE.replace('geodesics="true"', 'geodesics="true" denoise="true"')),
    ("w14-path-samples", ["W14"], lambda _: BLACKHOLE.replace('geodesics="true"', 'geodesics="true" pathSamples="5000"')),
    # 1920 × 1080 at 320 samples and 20 bounces: 6.25 times the 256 samples and 4 bounces W15 starts past
    ("w15-path-cost", ["W15"], lambda _: '<scene version="1.3"><project width="1920" height="1080" fps="24" duration="1"/><composition><camera id="eye" z="-10" renderer="pathtrace" pathSamples="320" maxBounces="20"/><object3D id="ball" primitive="sphere" radius="1"/></composition></scene>\n'),
    # the time-frames draft, rule 7: an ephemeris position replaces the object's own
    ("w16-ephemeris-xyz", ["W16"], lambda _: TF.replace('positionFrom="ephemeris"', 'positionFrom="ephemeris" x="12" z="-3"')),
    # the star-fields draft, SF4: a space's clock gives the field's epoch
    ("w17-star-field-epoch", ["W17"], lambda _: SF.replace('catalogue="bsc"/>', 'catalogue="bsc" epoch="2100"/>').replace('</scene>', '<space epoch="2451545.0" timeScale="tdb"/></scene>')),
    # the physical-sky draft, PSKY1: the physical sky's attributes under another sky
    ("w18-physical-attributes", ["W18"], lambda _: PHYS.replace('sky="physical"', 'sky="gradient"').replace(' sunSize="0.6"', '')),
    ("w05-lights", ["W05"], lambda _: BLACKHOLE.replace('</composition>', '</composition><lights><light id="sun" type="directional"/></lights>')),
    ("w06-foam-material", ["W06"], lambda _: OCEAN.replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('</ocean>', '<whitewater foamMode="albedo" foamMaterial="foam"/></ocean>').replace('<composition>', '<materials><material id="foam"/></materials><composition>')),
    ("w08-foam-unlit", ["W08"], lambda _: OCEAN.replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('<ocean id="sea" ', '<ocean id="sea" material="glow" ').replace('</ocean>', '<whitewater foamMode="albedo"/></ocean>').replace('<composition>', '<materials><material id="glow" unlit="true"/></materials><composition>')),
    ("w10-scatter-black-albedo", ["W10"], lambda _: VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#000000" scatterBounces="4" ')),
    ("w07-foam-raster", ["W07"], lambda _: OCEAN.replace('</ocean>', '<whitewater foamMode="albedo"/></ocean>')),
    ("w09-voxel-cells-small", ["W09"], lambda _: VOXELS.replace('cellSize="2"', 'cellSize="0.25"', 1)),
    ("w11-no-lens", ["W11"], lambda _: BLACKHOLE.replace(' geodesics="true"', '')),
    ("w01-non-finite", ["W01"], sub('<marker id="drop" time="4.2"', '<marker id="drop" time="4.2" duration="1"/>\n    <marker id="late" time="INF"')),
]

# SREPs 66 and 69: a build program in the composition, a data program feeding a repeat and a stepping program
PROGRAM_SHA = {k: hashlib.sha256((MEDIA / f"{k}.wasm").read_bytes()).hexdigest() for k in ("rect", "rows", "counter")}
PROGRAMS = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<parameters><program id="rows" src="../media/rows.wasm" sha256="' + PROGRAM_SHA["rows"] + '"/></parameters><composition>'
    '<program id="build" src="../media/rect.wasm" sha256="' + PROGRAM_SHA["rect"] + '" seed="3" fuel="100000"><param name="side" value="1"/></program>'
    '<repeat id="rep" over="rows"><shape id="c" shape="rect" width="4" height="4" fill="#FF0000"/></repeat>'
    '<program id="steps" mode="step" width="8" height="8" src="../media/counter.wasm" sha256="' + PROGRAM_SHA["counter"] + '" stepsPerFrame="2" prewarm="3"/>'
    '</composition></scene>\n')

# SREP 70: a parametric curve, a parametric surface and a heightfield, in version 1.6
PARAMETRIC = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<materials><material id="m" baseColor="#FF0000" unlit="true"/></materials><composition>'
    '<shape id="curve" shape="parametric" stroke="#FFFFFF" strokeWidth="2"><parametricPath x="32 + 20 * Math.cos(t)" y="32 + 10 * Math.sin(t)" t0="0" t1="6.283185307179586" samples="64" closed="true"/></shape>'
    '<object3D id="surface" primitive="parametric" material="m" x="32" y="32"><parametricSurface x="u" y="v" z="0" u0="-8" u1="8" v0="-4" v1="4" uSamples="4" vSamples="4"/></object3D>'
    '<object3D id="ground" primitive="heightfield" material="m" x="32" y="48" rotationX="90"><heightfield height="2 * Math.sin(x / 4)" width="16" depth="8" xSamples="8" zSamples="4"/></object3D>'
    '</composition></scene>\n')
VALID = {
    "solid-colliders": SOLID_COLLIDERS,
    "fracture": FRACTURE,
    "openvdb": VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/impact-0.vdb" format="openvdb" temperatureGrid="temperature"').replace('<medium ', '<medium blackbody="true" '),
    "openvdb-sequence": VOLUME.replace('src="../media/uniform.srvol"', 'src="../media/impact-%d.vdb" format="openvdb" first="0" last="1" interpolation="linear"'),
    "crater": CRATER,
    "crater-impact": CRATER_IMPACT,
    "pyro-crater": PYRO_CRATER,
    "ejecta-crater": EJECTA_CRATER,
    "pyro-crater-push": PYRO_CRATER.replace('<pyroSource crater="pit"/>', '<pyroSource crater="pit" velocityRateY="-2"/>').replace('<pyroImpulse crater="pit" heatFraction="0.2"/>', '<pyroImpulse crater="pit" heatFraction="0.2" velocityX="1"/>'),
    "ocean-entry": OCEAN_ENTRY,
    "ocean-hydrostatic": OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="hydrostatic" '),
    "ocean-depth-filtered-drag": OCEAN_COUPLED.replace('<ocean ', '<ocean bedResponse="depthFiltered" bodyDrag="2" '),
    "model-select": MODEL_SELECT,
    "repeat-points": POINTS,
    "programs": PROGRAMS,
    "parametric": PARAMETRIC,
    "repeat-points-neutral-steps": POINTS.replace('<repeat id="r"', '<repeat id="r" from="0" step="1"'),
    "connector": CONNECTOR,
    "connector-points": CONNECTOR.replace(' to="b"', ' toX="50%" toY="100%"').replace('route="orthogonal"', 'route="curved" bend="-20"').replace(' label="note"', ''),
    "pdf-region": PDF,
    "stroke-text": STROKE_TEXT,
    "pinned-fonts": PINNED_FONTS,
    "morph": MORPH,
    "joint": JOINT,
    "text3d-tracking": TEXT3D,
    "mesh-sequence": MESH_SEQUENCE,
    "globe-relief": GLOBE,
    "particles3d": PARTICLES3D,
    "ocean-colliders": OCEAN_COUPLED,
    "ocean-density": OCEAN_BUOYANCY.replace('<ocean ', '<ocean density="1030" '),
    "ocean-buoyancy": OCEAN_BUOYANCY,
    "physics-internal-edges": OCEAN_BUOYANCY.replace('<physics pixelsPerMeter="1"/>', '<physics pixelsPerMeter="1" fixInternalEdges="true"/>'),
    "fracture-contact": FRACTURE_CONTACT,
    "fracture-contact-tuned": FRACTURE_CONTACT.replace('<fracture ', '<fracture minImpulse="50" energyFraction="0.5" '),
    "crater-impact-influence-small": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" influenceDepth="5"'),
    "crater-impact-influence-large": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" influenceDepth="30"'),
    "black-hole": BLACKHOLE,
    "black-hole-without-disk": BLACKHOLE.replace('<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>', ''),
    "black-hole-disk-at-isco": BLACKHOLE.replace('<accretionDisk id="disk"', '<accretionDisk id="disk" innerRadius="6" seed="3" angularPattern="spiral" contrast="0.3" intensity="2" timeScale="0.5" rotationX="20" rotationY="5" rotation="-10"'),
    "crater-mantle": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true"'),
    "crater-repose": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" repose="35"'),
    "crater-mantle-bulking": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="true" bulking="1"'),
    "crater-capture": CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" capture="true"'),
    "ocean-splash": EJECTA_CRATER.replace('</composition>', '<ocean id="sea" width="16" depth="16" cellSize="1" splash="debris"/></composition>'),
    "particles3d-gas": PARTICLES3D.replace('<composition>', '<composition><object3D id="smoke" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" boundary="open"/></object3D>').replace('<particles3D id="dust"', '<particles3D id="dust" drag="1" gas="smoke"'),
    "ocean-full": OCEAN_BUOYANCY.replace('bodyCoupling="buoyancy"', 'bodyCoupling="full"'),
    "ocean-order2": OCEAN.replace('bottomDepth="2"', 'bottomDepth="2" order="2"'),
    "ocean": OCEAN.replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3"/></ocean>'),
    "whitewater-checkpoint": OCEAN.replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3" checkpointMemoryMiB="0"/></ocean>'),
    "whitewater-foam-albedo": OCEAN.replace('</composition>', '<camera id="cam" renderer="pathtrace"/></composition>').replace('</ocean>', '<whitewater emissionRate="2" threshold="0.3" foamMode="albedo" foamRadius="0.75" foamAlbedo="0.9" foamRoughness="0.8"/></ocean>'),
    "pyro": PYRO,
    "pyro-multigrid": PYRO.replace('<pyro ', '<pyro solver="multigrid" '),
    "pyro-maccormack": PYRO.replace('<pyro ', '<pyro solver="multigrid" advection="maccormack" '),
    "pyro-mesh": PYRO_MESH,
    "pyro-follow": PYRO_FOLLOW,
    "pyro-blast": PYRO_BLAST,
    "pyro-blast-gas": PYRO_BLAST.replace('<pyroBlast ', '<pyroBlast ambientDensity="0.9" ambientPressure="90000" gamma="1.67" '),
    "voxels": VOXELS,
    "voxels-from-mesh": VOXELS_FROM_MESH,
    # the extension is what says the format: a `.srvol` file with a grid, and one with the format said and no extension to say it
    "voxels-srvol": VOXELS.replace('src="../media/voxels.vox"', 'src="../media/uniform.srvol" voxelGrid="voxels"'),
    "voxels-surface-memory": VOXELS.replace('surface="blocks"', 'surface="blocks" surfaceMemoryMiB="64"'),
    "voxels-srvol-format": VOXELS.replace('src="../media/voxels.vox"', 'src="../media/uniform.srvol" format="srvol" voxelGrid="voxels"'),
    "voxel-body": VOXEL_BODY,
    "voxel-crater": VOXEL_GROUND,
    "voxel-ejecta": VOXEL_EJECTA,
    "voxel-crater-signed-scale": VOXEL_GROUND.replace('y="2">', 'y="2" scaleX="+3" scaleY="3" scaleZ=" 3.0 ">'),
    "voxel-fracture": VOXEL_FRACTURE,
    "voxel-fracture-stress": VOXEL_STRESS,
    "voxel-fracture-planes": VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="1 0 0 2 0 1 0 0.5"'),
    "voxel-fracture-labels": VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="labels" labels="material"'),
    "pyro-colliders": PYRO_COLLIDERS,
    "pyro-fields": PYRO.replace('<pyro ', '<pyro forceFields="wind" useForceFields="true" ').replace('</scene>', '<physics><forceField id="wind" type="wind" forceX="1" affects="particles" start="0.1" end="0.8"/></physics></scene>'),
    "baked-volume": VOLUME_BAKED,
    "volume": VOLUME,
    "volume-scatter-bounces": VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#808080" scatterBounces="4" '),
    "volume-scatter-bounces-zero": VOLUME.replace('<medium albedo="#000000" ', '<medium albedo="#808080" scatterBounces="0" '),
    "volume-light-grid": VOLUME.replace('<medium ', '<medium lighting="grid" lightGridCell="2" lightGridDomeDirections="32" lightGridMemoryMiB="64" '),
    "advected-volume": VOLUME_SEQUENCE.replace('uniform-%02d', 'advected-%d').replace('last="0"', 'last="1"').replace('fps="24000/1001"', 'fps="1"').replace('interpolation="linear"', 'interpolation="advect" velocityGridX="velocity.x" velocityGridY="velocity.y" velocityGridZ="velocity.z"').replace(' boundsMinX="0" boundsMinY="0" boundsMinZ="0" boundsMaxX="32" boundsMaxY="32" boundsMaxZ="2"', '').replace('<composition>', '<composition><camera id="camera" x="0" y="0" z="-30" projection="orthographic"/>'),
    "volume-sequence": VOLUME_SEQUENCE.replace('first="0" last="0"', 'first=" +0 " last=" 0000 "').replace('uniform-%02d', 'uniform-0%d').replace('</assets>', '<volume id="hash" src="../media/uniform-##.srvol" first="0" last="0"/></assets>'),
    "thermal-volume": VOLUME.replace('id="smoke" src=', 'id="smoke" temperatureGrid="density" src=').replace('<medium ', '<medium blackbody="true" temperatureScale="5000" '),
    "kitchen-sink": base,
    "minimal": MINIMAL,
    "version-1.0": v10('<layer id="l" asset="img" x="10" y="10"/>').replace("<composition>", '<assets><image id="img" src="../media/logo.png" width="16" height="16"/></assets><composition>'),
}

# libxml2 (the oracle) skips two XSD 1.0 checks that sr-model enforces:
# IDREF resolution (Part 1, Validation Rule: Validation Root Valid (ID/IDREF
# Table)) and minLength 1 on whitespace-only IDREFS lists.
ORACLE_BLIND = {"s06-idrefs-empty", "s10-dangling-idref", "r53-empty-list"}

# ---- generated marker ids (beat.N and bar.N of a beatGrid), SREP 57
def marker_doc(markers='<beatGrid bpm="120" offset="0.25"/>', shape='', anim='', outputs=''):
    return f"""<scene version="1.1">
  <project width="480" height="270" fps="24" duration="4" background="#000000" motionBlur="false" seed="1"/>
{outputs}
  <markers>{markers}</markers>
  <composition>
    <shape id="n" shape="rect" x="100" y="100" width="60" height="60" fill="#FF0000" {shape}>{anim}</shape>
  </composition>
</scene>
"""

KEY_ANIM = '<animate property="x"><key time="0" value="0"/><key time="1.9" value="400" marker="{}"/></animate>'
POSTER = '\n  <output id="o" path="out/o.mp4" codec="h264"><poster path="out/p.png" marker="{}"/></output>'
VALID.update({
    "gm-start": marker_doc(shape='startMarker="beat.3"'),
    "gm-bar": marker_doc(shape='startMarker="bar.2"'),
    "gm-end": marker_doc(shape='startMarker="beat.0" endMarker="bar.1"'),
    "gm-key": marker_doc(anim=KEY_ANIM.format("beat.4")),
    "gm-poster": marker_doc(outputs=POSTER.format("bar.1")),
    "gm-leading-zeros": marker_doc(shape='startMarker="beat.007"'),
    "gm-explicit": marker_doc('<beatGrid bpm="120" offset="0.25"/><marker id="m3" time="1.25"/>', shape='startMarker="m3"'),
    "gm-name-without-grid": marker_doc('<marker id="beat.3" time="1.5"/>', shape='startMarker="beat.3"'),
})
CASES += [
    ("gm-no-grid", ["R21"], lambda _: marker_doc('<marker id="m3" time="1.25"/>', shape='startMarker="beat.3"')),
    ("gm-malformed-empty", ["R21"], lambda _: marker_doc(shape='startMarker="beat."')),
    ("gm-malformed-letters", ["R21"], lambda _: marker_doc(shape='startMarker="beat.x"')),
    ("gm-malformed-dots", ["R21"], lambda _: marker_doc(shape='startMarker="beat.3.1"')),
    ("gm-malformed-kind", ["R21"], lambda _: marker_doc(shape='startMarker="beats.3"')),
    ("gm-too-many-digits", ["R21"], lambda _: marker_doc(shape='startMarker="beat.12345678901234567890"')),
    ("gm-element-id", ["R21"], lambda _: marker_doc(shape='startMarker="n"')),
]
WARN_CASES += [
    ("w04-poster-no-grid", ["W04"], lambda _: marker_doc('<marker id="m3" time="1.25"/>', outputs=POSTER.format("bar.1"))),
    ("w04-poster-element-id", ["W04"], lambda _: marker_doc(outputs=POSTER.format("n"))),
    ("w04-poster-dangling", ["W04"], lambda _: marker_doc(outputs=POSTER.format("nowhere"))),
]
WARN_CASES.append(
    ("w03-marker-named-like-generated", ["W03"],
     lambda _: marker_doc('<beatGrid bpm="120" offset="0.25"/><marker id="beat.3" time="1.25"/>', shape='startMarker="beat.3"'))
)

CASES.append(("openvdb-format", ["S06"], lambda _: VALID["openvdb"].replace('format="openvdb"', 'format="guess"')))

CASES += [
    ("r53-empty-list", ["S06", "R53"], lambda _: MINIMAL.replace('<composition/>', '<composition><group id="g" effects=""/></composition>')),
    ("r54-audiogram-source", ["R54"], lambda _: MINIMAL.replace('<composition/>', '<assets><audiogram id="a" source="wrong" width="32" height="32"/></assets><composition><group id="wrong"/></composition>')),
]

# SREP 67 (document version 1.6): compute, iterate and the serial effect types
SREP67 = '<scene version="1.6"><project width="64" height="64" fps="10" duration="1"/><composition>{body}</composition>{post}</scene>\n'
SREP67_SHAPE = '<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="fx"/>'
CASES += [
    ("v13-iterate", ["V13"], lambda _: SREP67.format(body=f'<iterate id="it" steps="10">{SREP67_SHAPE}</iterate>', post='<effects><effect id="fx" type="blur"/></effects>').replace('version="1.6"', 'version="1.5"')),
    ("itr1-check-every", ["ITR1"], lambda _: SREP67.format(body=f'<iterate id="it" steps="10" checkEvery="20">{SREP67_SHAPE}</iterate>', post='<effects><effect id="fx" type="blur"/></effects>')),
    ("itr2-nested", ["ITR2"], lambda _: SREP67.format(body=f'<iterate id="a" steps="10"><iterate id="b" steps="5">{SREP67_SHAPE}</iterate></iterate>', post='<effects><effect id="fx" type="blur"/></effects>')),
    ("srt1-low-high", ["SRT1"], lambda _: SREP67.format(body=SREP67_SHAPE, post='<effects><effect id="fx" type="segmented-sort" low="0.8" high="0.2"/></effects>')),
    ("cmp13-two-tonemaps", ["CMP13"], lambda _: SREP67.format(body='<compute id="c" src="../media/srep67-three.wgsl" width="4" height="4" invocations="1"><tonemap/><tonemap/></compute>', post='')),
]

# SREP 68: stepsPerFrame and prewarm apply to shader effects only
CASES += [
    ("stp1-not-a-shader", ["STP1"], lambda _: '<scene version="1.6"><project width="64" height="64" fps="10" duration="1"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="fx"/></composition><effects><effect id="fx" type="blur" stepsPerFrame="2"/></effects></scene>\n'),
]

# SREP 70: the version gate V14, PAR1 and PAR2, and C69's exemption of parametric shapes
CASES += [
    ("v14-path", ["V14"], lambda _: PARAMETRIC.replace('version="1.6"', 'version="1.5"')),
    ("v14-heightfield", ["V14"], lambda _: re.sub(r'<shape id="curve".*?</shape>|<object3D id="surface".*?</object3D>', '', PARAMETRIC).replace('version="1.6"', 'version="1.2"')),
    ("par1-missing", ["PAR1"], lambda _: re.sub(r'<parametricPath [^>]*/>', '', PARAMETRIC)),
    ("par1-other-shape", ["PAR1"], lambda _: PARAMETRIC.replace('shape="parametric"', 'shape="rect" width="4" height="4"')),
    ("par1-two", ["PAR1"], lambda _: re.sub(r'(<parametricPath [^>]*/>)', r'\1\1', PARAMETRIC)),
    ("par2-missing", ["PAR2"], lambda _: re.sub(r'<parametricSurface [^>]*/>', '', PARAMETRIC)),
    ("par2-other-primitive", ["PAR2"], lambda _: PARAMETRIC.replace('primitive="heightfield"', 'primitive="plane"')),
    ("par2-crossed", ["PAR2"], lambda _: PARAMETRIC.replace('primitive="parametric"', 'primitive="heightfield"')),
]

# SREP 73: flock/@orientToVelocity; false on a streak flock (the default shape) is inert (INERT-I16, information)
FLOCK_UPRIGHT = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<composition><flock id="f" width="32" height="32" count="4" seed="2" shape="disc" orientToVelocity="false"/></composition></scene>\n')
VALID["flock-upright"] = FLOCK_UPRIGHT
# Two 3D mistakes that validate cannot see and the render report finds over the frames rendered: an object3D never inside
# the camera's view ("behind" sits behind the default camera; "seen" is in front of it), and a metal with no environment
# image to reflect. Valid documents: their findings are the render report's (crates/sr-deliver/tests/silent_3d.rs).
VALID["object3d-off-camera"] = ('<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/><composition>'
    '<object3D id="seen" primitive="box" width="20" height="20" depth="20" x="32" y="32"/>'
    '<object3D id="behind" primitive="sphere" radius="10" x="32" y="32" z="-1000"/></composition></scene>\n')
VALID["metal-without-environment"] = ('<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/>'
    '<materials><material id="chrome" baseColor="#D0D0D0" metallic="1" roughness="0.15"/></materials><composition>'
    '<object3D id="ball" primitive="sphere" radius="20" x="32" y="32" material="chrome"/></composition></scene>\n')
WARN_CASES += [
    ("inert-i16-streak-flock", ["INERT-I16"], lambda _: FLOCK_UPRIGHT.replace(' shape="disc"', '')),
    ("inert-i17-zdepth-without-threed", ["INERT-I17"], lambda _: MINIMAL.replace('<composition/>', '<composition><shape id="s" shape="rect" width="10" height="10" zDepth="5"/></composition>')),
    ("inert-i17-margin-without-align", ["INERT-I17"], lambda _: MINIMAL.replace('<composition/>', '<composition><shape id="s" shape="rect" width="10" height="10" margin="8"/></composition>')),
]

# SREP 71: subsurface, the procedural sky and generator-fed maps; no version gate (new attributes, neutral defaults)
SREP71 = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<assets><generator id="g" kind="noise" width="16" height="16"/><image id="im" src="../media/logo.png" width="16" height="16"/></assets>'
    '<materials><material id="m" subsurface="0.5" subsurfaceColor="#E0F0D0FF" subsurfaceRadius="20" subsurfaceRadiusScale="1 0.5 0.25" {maps}/></materials>'
    '<composition><object3D id="o" primitive="sphere" radius="20" x="32" y="32" material="m"/></composition>'
    '<lights><light id="d" type="dome" sky="gradient" skyZenith="#2050A0" skyHorizon="#C0D0E0" skyGround="#403020" skyExponent="1" '
    'sunAzimuth="10" sunElevation="5" sunSize="5" sunColor="#FF0000" sunIntensity="1" environmentVisible="true"/>{lights}</lights></scene>\n')
VALID["srep71-subsurface-sky-maps"] = SREP71.format(maps='baseColorMap="#g" normalMap="#im" displacementMap="#g" displacementScale="2"', lights='')
CASES += [
    ("sky1-not-dome", ["SKY1"], lambda _: SREP71.format(maps='', lights='<light id="k" type="directional" sky="gradient"/>')),
    ("sky2-environment", ["SKY2"], lambda _: SREP71.format(maps='', lights='').replace('environmentVisible="true"', 'environment="../media/studio.hdr"')),
]
for _attr in ["baseColorMap", "normalMap", "metallicRoughnessMap", "occlusionMap", "emissiveMap", "displacementMap"]:
    CASES.append((f"mtx1-{_attr.lower()}", [f"MTX1-{_attr}"], lambda _, a=_attr: SREP71.format(maps=f'{a}="#m"', lights='')))

# SREP 74: a viewport3D (document version 1.6), V15, VP1 and VP3; and BH5, BH6 and BH8, which keep to the scope of the
# camera (the viewport3D it is in, else the document outside every viewport3D). A black hole needs version 1.3 (BH1) and
# a viewport3D 1.6 (V15), so each black-hole case also reports V15.
VIEWPORT = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<materials><material id="m" baseColor="#FF0000" unlit="true" doubleSided="true"/></materials>'
    '<composition>'
    '<viewport3D id="v" width="32" height="16" x="8" y="8" camera="vc" lights="key" background="#202020FF">'
    '<camera id="vc" x="16" y="8" z="-27.7128"/>'
    '<object3D id="p" primitive="plane" width="8" height="4" x="16" y="8" material="m"/>'
    '</viewport3D>'
    '</composition><lights><light id="key" type="directional"/></lights></scene>\n')
VALID["viewport3d"] = VIEWPORT
VIEWPORT_IN_BLACKHOLE = lambda inner: BLACKHOLE.replace('<blackHole', f'<viewport3D id="v" width="16" height="16">{inner}</viewport3D><blackHole')
CASES += [
    ("v15-viewport", ["V15"], lambda _: VIEWPORT.replace('version="1.6"', 'version="1.5"')),
    ("vp1-camera-outside", ["VP1"], lambda _: VIEWPORT.replace('<viewport3D', '<camera id="out" x="0" y="0" z="-50"/><viewport3D').replace('camera="vc"', 'camera="out"')),
    ("vp3-body", ["VP3"], lambda _: VIEWPORT.replace('material="m"/>', 'material="m"><rigidBody mass="1"/></object3D>')),
    ("bh6-viewport-objects-isolated", ["V15"], lambda _: VIEWPORT_IN_BLACKHOLE('<object3D id="ball" primitive="sphere" radius="1"/><particles3D id="dust" rate="1"/>')),
    ("bh5-viewport-camera-sees-no-hole", ["BH5", "V15"], lambda _: VIEWPORT_IN_BLACKHOLE('<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/>')),
    ("bh6-viewport-camera-sees-its-objects", ["BH5", "BH6", "V15"], lambda _: VIEWPORT_IN_BLACKHOLE('<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><object3D id="ball" primitive="sphere" radius="1"/>')),
    ("bh8-per-viewport", ["BH5", "BH8", "V15"], lambda _: VIEWPORT_IN_BLACKHOLE('<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><camera id="eye3" x="0" y="0" z="-90" geodesics="true"/>')),
]

# SREP 75: fractal assets (document version 1.6), V16 and FRC1. The centre is an xs:decimal, which sr-model reads at
# any length; libxml2 (the oracle) refuses one of more than 24 digits, a limit XSD 1.0 allows (Part 2, 3.2.3: at least
# 18), so these documents keep to 24 and the SREP's 33-digit centres are tested by sr-eval and sr-gpu only.
FRACTAL = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1"/>'
    '<assets><fractal id="f" kind="mandelbrot" width="16" height="16" centerX="-0.743643887037158704752191" '
    'centerY="+.131825904205311970493132" zoom="6" maxIterations="100" colorMode="bands" palette="#FF0000FF #0000FFFF">'
    '<animate property="zoom"><key time="0" value="6"/><key time="1" value="7"/></animate></fractal>'
    '<fractal id="j" kind="julia" width="16" height="16" juliaX="-0.8" juliaY="0.156"/></assets>'
    '<composition><layer id="l" asset="f"/><layer id="m" asset="j" x="20"/></composition></scene>\n')
VALID["fractal"] = FRACTAL
VALID["med-fog"] = VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phase="fog" dropletDiameter="20" anisotropy="0" ')
VALID["med-draine"] = VOLUME.replace('version="1.3"', 'version="1.8"').replace('<medium ', '<medium phase="draine" anisotropy="0.5" phaseAlpha="2" ')
PLOTS = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/>'
    '<composition><axes id="ax" width="48" height="32" x="8" y="16" xMin="-3.14" xMax="3.14" yMin="-1.5" yMax="1.5" grid="true">'
    '<plot kind="explicit" y="Math.sin(x)"/><plot kind="parametric" x="Math.cos(t)" y="Math.sin(t)" t1="6.28" closed="true"/>'
    '<plot kind="polar" r="0.5"/><plot kind="implicit" f="x * x + y * y - 1"/>'
    '<shape id="dot" shape="ellipse" width="4" height="4" x="0" y="0" fill="#FF0000FF"/></axes></composition></scene>\n')
PLOTS3D = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/>'
    '<materials><material id="m" baseColor="#FFCC00FF"/></materials><composition>'
    '<object3D id="o" primitive="axes"><axes3D width="40" height="40" depth="40"/>'
    '<plot3D x="Math.cos(t)" y="Math.sin(t)" z="t / 6.28" t1="12.56" material="m"/></object3D></composition></scene>\n')
VALID["plots"] = PLOTS
VALID["plots-3d"] = PLOTS3D
CASES += [
    ("v17", ["V17"], lambda _: PLOTS.replace('version="1.8"', 'version="1.7"')),
    ("v17-axes3d", ["V17"], lambda _: PLOTS3D.replace('version="1.8"', 'version="1.6"')),
    ("plt1", ["PLT1"], lambda _: PLOTS.replace('<plot kind="polar" r="0.5"/>', '<plot kind="polar" r="0.5" y="1"/>')),
    ("plt2", ["PLT2"], lambda _: PLOTS.replace(' yMin="-1.5" yMax="1.5"', ' yMin="2"')),
    ("plt2-axes3d", ["PLT2"], lambda _: PLOTS3D.replace('<axes3D ', '<axes3D zMin="1" zMax="0" ')),
    ("plt3", ["PLT3"], lambda _: PLOTS3D.replace('<axes3D width="40" height="40" depth="40"/>', '')),
    ("plt3-plot3d", ["PLT3"], lambda _: PLOTS3D.replace('primitive="axes"><axes3D width="40" height="40" depth="40"/>', 'primitive="sphere">')),
    ("plot-kind-enum", ["S06", "PLT1"], lambda _: PLOTS.replace('kind="explicit"', 'kind="cubic"')),
    ("plot-resolution-range", ["S06"], lambda _: PLOTS.replace('f="x * x + y * y - 1"', 'f="x * x + y * y - 1" resolution="5000"')),
    ("plot3d-segments-range", ["S06"], lambda _: PLOTS3D.replace('material="m"', 'material="m" tubeSegments="2"')),
]
SPACE = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><composition>'
    '<shape id="s" shape="rect" width="40" height="30" fill="#FFFFFFFF"><deform><modifier type="matrix" b="0.5" e="4"/>'
    '<modifier type="map" x="u + 0.1 * v * v" y="v" unit="20"/><modifier type="complex" function="mobius" coefficients="1 0 0 0 0 0.01 1 0"/></deform></shape>'
    '<group id="g"><deform><modifier type="complex" function="square" unit="40"/></deform>'
    '<shape id="l" shape="rect" width="20" height="20" stroke="#FF0000FF" strokeWidth="2"/></group></composition></scene>\n')
VALID["space-transforms"] = SPACE
CASES += [
    ("v17-space", ["V17"], lambda _: SPACE.replace('version="1.8"', 'version="1.7"')),
    ("spt1", ["SPT1"], lambda _: SPACE.replace(' x="u + 0.1 * v * v"', '')),
    ("spt1-mobius", ["SPT1"], lambda _: SPACE.replace(' coefficients="1 0 0 0 0 0.01 1 0"', '')),
    ("spt2", ["SPT2"], lambda _: SPACE.replace('coefficients="1 0 0 0 0 0.01 1 0"', 'coefficients="1 0 2 0 1 0 2 0"')),
    ("space-function-enum", ["S06"], lambda _: SPACE.replace('function="square"', 'function="tan"')),
    ("space-group-body", ["S02"], lambda _: SPACE.replace('<group id="g"><deform>', '<group id="g"><rigidBody/><deform>')),
]
TEX = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><assets>'
    '<formula id="f" tex="\\part{a}{x^2} + \\part{b}{2x} + \\part{c}{1}" width="64" height="32"/>'
    '<formula id="g" tex="\\part{a}{x^2} + \\part{d}{2x + 1}" width="64" height="32"/>'
    '<formulaMorph id="m" from="f" to="g" match="name" width="64" height="32"><animate property="progress"><key time="0" value="0"/><key time="1" value="1"/></animate></formulaMorph>'
    '</assets><composition><layer id="lf" asset="f"><formulaAnimator part="b" y="-4"/></layer><layer id="lm" asset="m"/>'
    '<shape id="s" shape="path" morph="true" width="20" height="20" path="M 0 0 L 20 0 L 20 20 L 0 20 Z" fill="#FF0000FF">'
    '<animate property="path"><key time="0" value="M 0 0 L 20 0 L 20 20 L 0 20 Z"/><key time="1" value="M 0 0 L 20 0 L 10 16 Z"/></animate></shape>'
    '</composition></scene>\n')
VALID["semantic-tex"] = TEX
CASES += [
    ("v17-semantic-tex", ["V17"], lambda _: TEX.replace('version="1.8"', 'version="1.7"')),
    ("v17-part", ["V17"], lambda _: TEX.replace('version="1.8"', 'version="1.7"').split('<formulaMorph')[0] + '</assets><composition/></scene>\n'),
    ("frm1", ["FRM1"], lambda _: TEX.replace('<layer id="lm" asset="m"/>', '<layer id="lm" asset="m"><formulaAnimator part="a"/></layer>')),
    ("frm2", ["FRM2"], lambda _: TEX.replace('<formulaMorph id="m" from="f"', '<formulaMorph id="m" from="lf"')),
    ("formula-morph-match-enum", ["S06"], lambda _: TEX.replace('match="name"', 'match="glyph"')),
    ("formula-animator-part-required", ["S05"], lambda _: TEX.replace('<formulaAnimator part="b"', '<formulaAnimator')),
]
RV = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><assets>'
    '<value id="x" step="0.25" update="value + 2 * dt" between="linear"/>'
    '<value id="m" count="8" update="value + draw(0, index) / count" seed="3"/>'
    '<value id="k"><animate property="value"><key time="0" value="0"/><key time="1" value="4"/></animate></value>'
    '<chart id="c" kind="counter" width="64" height="32"><series name="x" values="0"><link property="values" source="x.value"/></series></chart>'
    '</assets><composition><layer id="lc" asset="c"/></composition></scene>\n')
VALID["reactive-values"] = RV
CASES += [
    ("v17-reactive-values", ["V17"], lambda _: RV.replace('version="1.8"', 'version="1.7"').replace('<link property="values" source="x.value"/>', '')),
    ("rv1-update-without-step", ["RV1"], lambda _: RV.replace(' step="0.25" update="value + 2 * dt" between="linear"', ' update="value + 2 * dt"')),
    ("rv2-linear-index-fold", ["RV2"], lambda _: RV.replace('count="8"', 'count="8" between="linear"')),
    ("value-count-bound", ["S06"], lambda _: RV.replace('count="8"', 'count="1000001"')),
]
FIELDS = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/>'
    '<parameters><data id="flow" format="csv">x,y,u,v\n0,0,1,0\n1,0,1,1\n0,1,0,1\n1,1,-1,1</data></parameters><composition>'
    '<field id="rot" u="-y" v="x" width="64" height="64" xMin="-2" xMax="2" yMin="-2" yMax="2">'
    '<arrows cols="8" rows="8"/><streamlines separation="12" maxLines="50"/><lic length="6" resolution="32" seed="5" opacity="0.5"/></field>'
    '<field id="sq" kind="complex" function="square" width="32" height="32"><portrait/></field>'
    '<field id="sad" kind="scalar" u="x * x - y * y" width="32" height="32"><contours levels="-0.5 0 0.5"/></field>'
    '<field id="pair" width="64" height="32"><charge x="-0.5" y="0" q="1"/><charge x="0.5" y="0" q="-1"/><lines perCharge="6"/></field>'
    '<field id="g" grid="flow" width="32" height="32" xMin="0" xMax="1" yMin="0" yMax="1"><arrows cols="2" rows="2"/></field>'
    '<field id="mob" kind="complex" function="mobius" coefficients="1 0 0 0 0 0 1 0" width="32" height="32"><portrait base="3"/></field>'
    '<axes id="ax" width="32" height="32"><field id="inax" u="1" v="0"><arrows cols="2" rows="2"/></field></axes>'
    '<field id="loop" width="64" height="32"><wire x1="-1" y1="0.5" x2="1" y2="0.5" q="1" i="1"/>'
    '<wire x1="1" y1="-0.5" x2="-1" y2="-0.5" q="-1" i="1"/><arrows cols="9" rows="9" show="poynting"/><arrows show="field"/></field>'
    '</composition></scene>\n')
VALID["fields"] = FIELDS
CASES += [
    ("v17-fields", ["V17"], lambda _: FIELDS.replace('version="1.8"', 'version="1.7"')),
    ("fld1", ["FLD1"], lambda _: FIELDS.replace('function="square"', 'function="square" w="x; y"')),
    ("fld1-v-scalar", ["FLD1"], lambda _: FIELDS.replace('u="x * x - y * y"', 'u="x * x - y * y" v="x"')),
    ("fld2", ["FLD2"], lambda _: FIELDS.replace('yMin="-2" yMax="2"', 'yMin="2" yMax="-2"')),
    ("fld3", ["FLD3"], lambda _: FIELDS.replace('<arrows cols="8" rows="8"/>', '<arrows cols="8" rows="8"/><portrait/>')),
    ("fld3-lines", ["FLD3"], lambda _: FIELDS.replace('<charge x="0.5" y="0" q="-1"/>', '').replace('<charge x="-0.5" y="0" q="1"/>', '<dipole x="0" y="0" mx="1" my="0"/>')),
    ("fld4", ["FLD4"], lambda _: FIELDS.replace('separation="12"', 'separation="12" test="13"')),
    ("fld4-max-steps", ["FLD4"], lambda _: FIELDS.replace('<lines perCharge="6"/>', '<lines perCharge="6" maxSteps="2000000"/>')),
    ("fld1-kind", ["FLD1"], lambda _: FIELDS.replace('kind="scalar" u="x * x - y * y"', 'kind="scalar" forceField="rot"')),
    ("fld1-vector-u-alone", ["FLD1"], lambda _: FIELDS.replace('u="-y" v="x"', 'u="-y"')),
    ("fld1-mobius-count", ["FLD1"], lambda _: FIELDS.replace('coefficients="1 0 0 0 0 0 1 0"', 'coefficients="1 0 0"')),
    ("fld1-coefficients-not-mobius", ["FLD1"], lambda _: FIELDS.replace('function="square"', 'function="square" coefficients="1 0 0 0 0 0 1 0"')),
    ("fld5", ["FLD5"], lambda _: FIELDS.replace('<field id="sq" kind="complex" function="square" width="32" height="32">', '<field id="sq" kind="complex" function="square" width="32">')),
    ("field-portrait-base", ["S06"], lambda _: FIELDS.replace('<portrait base="3"/>', '<portrait base="1"/>')),
    ("field-kind-enum", ["S06", "FLD1", "FLD3"], lambda _: FIELDS.replace('kind="complex" function="square"', 'kind="tensor" function="square"')),
    ("field-arrows-bound", ["S06"], lambda _: FIELDS.replace('cols="8"', 'cols="1001"')),
    ("fld6-no-current", ["FLD6"], lambda _: FIELDS.replace('q="-1" i="1"/>', 'q="-1"/>').replace('q="1" i="1"/>', 'q="1" i="0"/>')),
    ("fld6-no-charge", ["FLD6"], lambda _: FIELDS.replace('q="-1" i="1"/>', 'i="1"/>').replace('q="1" i="1"/>', 'q="0" i="1"/>')),
    ("fld1-wire-with-u", ["FLD1"], lambda _: FIELDS.replace('<field id="loop" width="64" height="32">', '<field id="loop" u="x" v="y" width="64" height="32">')),
    ("field-arrows-show-enum", ["S06"], lambda _: FIELDS.replace('show="poynting"', 'show="flux"')),
    ("field-source-order", ["S02"], lambda _: FIELDS.replace('<charge x="0.5" y="0" q="-1"/><lines perCharge="6"/>', '<lines perCharge="6"/><charge x="0.5" y="0" q="-1"/>')),
]
# the time-frames draft: space, the ephemeris asset, object3D/@body, @positionFrom and @orientation, TF2 to TF5, V17
TF = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><assets>'
    '<ephemeris id="de" src="../media/de440s-excerpt.bsp" digest="sha256:' + hashlib.sha256((MEDIA / "de440s-excerpt.bsp").read_bytes()).hexdigest() + '"/>'
    '</assets><composition><object3D id="moon" primitive="sphere" radius="4" body="301" positionFrom="ephemeris" orientation="bodyFixed"/></composition>'
    '<physics pixelsPerMeter="1e-6"/><space epoch="2451545.0" timeScale="tdb" origin="399" ephemeris="de"/></scene>\n')
TF_APPROX = TF.replace('body="301" positionFrom="ephemeris" orientation="bodyFixed"', 'body="4" positionFrom="ephemeris"').replace(' origin="399" ephemeris="de"', '').replace('epoch="2451545.0" timeScale="tdb"', 'epoch="2025-06-01T00:00:00Z"')
VALID["time-frames"] = TF
VALID["time-frames-approximate"] = TF_APPROX
VALID["time-frames-utc-offset"] = TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="2024-02-29T23:59:59.25+05:30"')
VALID["time-frames-utc-instant-1972"] = TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="1971-12-31T23:30:00-01:00"')
VALID["time-frames-utc-instant-3000"] = TF_APPROX.replace('epoch="2025-06-01T00:00:00Z"', 'epoch="3001-01-01T00:30:00+01:00"')
VALID["time-frames-body-fixed"] = TF.replace('timeScale="tdb"', 'timeScale="tt" rate="+86164.0962" frame="bodyFixed" frameBody="0399"')
CASES += [
    ("v17-time-frames", ["V17"], lambda _: TF.replace('version="1.8"', 'version="1.7"')),
    ("tf2-february-29", ["TF2"], lambda _: TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="2026-02-29T00:00:00Z"')),
    ("tf2-before-1972", ["TF2"], lambda _: TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="1972-01-01T00:30:00+01:00"')),
    ("tf2-offset-minutes", ["TF2"], lambda _: TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="2026-10-24T00:00:00+05:60"')),
    ("tf2-fraction-without-digits", ["TF2"], lambda _: TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="2026-10-24T00:00:00.Z"')),
    ("tf2-non-ascii", ["TF2"], lambda _: TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="2026-10-24T00:00:00+0\u00e9:00"')),
    ("tf2-julian-exponent", ["TF2"], lambda _: TF.replace('epoch="2451545.0"', 'epoch="2.451545e6"')),
    ("tf2-utc-instant-1971", ["TF2"], lambda _: TF.replace('epoch="2451545.0" timeScale="tdb"', 'epoch="1971-12-31T22:59:59-01:00"')),
    ("tf2-utc-instant-3001", ["TF2"], lambda _: TF_APPROX.replace('epoch="2025-06-01T00:00:00Z"', 'epoch="3000-12-31T23:30:00-01:00"')),
    ("tf2-approximate-year", ["TF2"], lambda _: TF_APPROX.replace('epoch="2025-06-01T00:00:00Z"', 'epoch="2817152.5" timeScale="tdb"')),
    ("tf3-no-frame-body", ["TF3"], lambda _: TF.replace('ephemeris="de"/>', 'ephemeris="de" frame="bodyFixed"/>')),
    ("tf3-frame-body-without-rotation", ["TF3"], lambda _: TF.replace('ephemeris="de"/>', 'ephemeris="de" frame="bodyFixed" frameBody="4"/>')),
    ("tf4-no-body", ["TF4"], lambda _: TF.replace(' body="301"', '')),
    ("tf4-no-space", ["TF4"], lambda _: TF.replace('<space epoch="2451545.0" timeScale="tdb" origin="399" ephemeris="de"/>', '')),
    ("tf4-approximate-moon", ["TF4"], lambda _: TF_APPROX.replace('body="4"', 'body="301"')),
    ("tf4-orientation-without-rotation", ["TF4"], lambda _: TF.replace('body="301"', 'body="3"')),
    ("tf5-rate-inf", ["TF5", "W01"], lambda _: TF.replace('timeScale="tdb"', 'timeScale="tdb" rate="INF"')),
    ("tf5-unknown-ephemeris", ["TF5"], lambda _: TF.replace('ephemeris="de"/>', 'ephemeris="dx"/>')),
    ("tf5-approximate-origin", ["TF5"], lambda _: TF_APPROX.replace('<space ', '<space origin="399" ')),
    ("space-time-scale-enum", ["S06", "TF2"], lambda _: TF.replace('timeScale="tdb"', 'timeScale="tai"')),
    ("ephemeris-digest-pattern", ["S06"], lambda _: re.sub(r'digest="sha256:([0-9a-f]{64})"', lambda m: 'digest="sha256:' + m.group(1).upper() + '"', TF)),
    ("object3d-position-from-enum", ["S06"], lambda _: TF.replace('positionFrom="ephemeris"', 'positionFrom="file"')),
]
# the star-fields draft: the catalogue asset, a dome light's starField, SF2 to SF4, V17
SF = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1" background="#000000FF"/><assets>'
    '<catalogue id="bsc" src="../media/bsc5-compact.csv.gz" digest="sha256:' + hashlib.sha256((MEDIA / "bsc5-compact.csv.gz").read_bytes()).hexdigest() + '"/>'
    '</assets><composition><camera id="eye" z="-10"/></composition><lights><light id="sky" type="dome"><starField catalogue="bsc"/></light></lights></scene>\n')
VALID["star-fields"] = SF
VALID["star-fields-labels"] = SF.replace('<starField catalogue="bsc"/>', '<starField catalogue="bsc" limitMagnitude="4.5" magnitudeZero="0.5" pointSize="2" properMotion="false" epoch="+2500" constellations="labels"/>')
CASES += [
    ("v17-star-fields", ["V17"], lambda _: SF.replace('version="1.8"', 'version="1.7"')),
    ("sf2-not-dome", ["SF2"], lambda _: SF.replace('type="dome"', 'type="directional"')),
    ("sf2-two-fields", ["SF2"], lambda _: SF.replace('<starField catalogue="bsc"/>', '<starField catalogue="bsc"/><starField catalogue="bsc" pointSize="3"/>')),
    ("sf2-not-a-catalogue", ["SF2"], lambda _: SF.replace('catalogue="bsc"/>', 'catalogue="eye"/>')),
    ("sf3-limit-nan", ["SF3", "W01"], lambda _: SF.replace('catalogue="bsc"/>', 'catalogue="bsc" limitMagnitude="NaN"/>')),
    ("sf4-epoch-range", ["SF4"], lambda _: SF.replace('catalogue="bsc"/>', 'catalogue="bsc" epoch="12000.5"/>')),
    ("star-field-constellations-enum", ["S06"], lambda _: SF.replace('catalogue="bsc"/>', 'catalogue="bsc" constellations="lines"/>')),
    ("catalogue-digest-pattern", ["S06"], lambda _: re.sub(r'digest="sha256:([0-9a-f]{64})"', lambda m: 'digest="' + m.group(1) + '"', SF)),
]
# the physical-sky draft: sky="physical" and its attributes, PSKY2 to PSKY4, V17 (PSKY1 is the warning W18)
PHYS = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/><composition><camera id="eye" z="-10"/></composition>'
    '<lights><light id="sky" type="dome" sky="physical" date="2003-10-17T12:30:30-07:00" latitude="39.742476" longitude="-105.1786" '
    'altitude="1830.14" pressure="820" temperature="11" deltaT="67" sunSize="0.6"/></lights></scene>\n')
VALID["physical-sky"] = PHYS
VALID["physical-sky-knobs"] = PHYS.replace(' sunSize="0.6"', ' sunSize="0.6" sunIntensity="2" moon="false" moonIntensity="0.5" atmosphere="none" aerosol="2" ozone="0" planetRadius="3389500" atmosphereHeight="+11.1e3" groundAlbedo="1" sunIrradiance="#FFE0C0FF" aerialPerspective="false" fogDensity="0.001" fogHeight="250"')
VALID["physical-sky-date-forms"] = PHYS.replace('2003-10-17T12:30:30-07:00', '2100-12-31T23:59:60.25Z').replace(' deltaT="67"', '')
VALID["physical-sky-ancient"] = PHYS.replace('2003-10-17T12:30:30-07:00', '-2000-03-21T06:00:00+05:30').replace('deltaT="67"', 'deltaT="+8000"')
CASES += [
    ("v17-physical-sky", ["V17"], lambda _: PHYS.replace('version="1.8"', 'version="1.7"')),
    ("psky2-no-date", ["PSKY2"], lambda _: PHYS.replace(' date="2003-10-17T12:30:30-07:00"', '')),
    ("psky2-no-seconds", ["PSKY2"], lambda _: PHYS.replace('12:30:30-07:00', '12:30-07:00')),
    ("psky2-no-zone", ["PSKY2"], lambda _: PHYS.replace('12:30:30-07:00', '12:30:30')),
    ("psky2-zone-form", ["PSKY2"], lambda _: PHYS.replace('12:30:30-07:00', '12:30:30-0700')),
    ("psky2-empty-fraction", ["PSKY2"], lambda _: PHYS.replace('12:30:30-07:00', '12:30:30.Z')),
    ("psky2-month", ["PSKY2"], lambda _: PHYS.replace('2003-10-17', '2003-13-17')),
    ("psky2-hour", ["PSKY2"], lambda _: PHYS.replace('T12:30:30', 'T24:30:30')),
    ("psky2-year", ["PSKY2"], lambda _: PHYS.replace('2003-10-17', '6001-10-17')),
    ("psky2-deltat-required", ["PSKY2"], lambda _: PHYS.replace('2003-10-17', '1899-10-17').replace(' deltaT="67"', '')),
    ("psky3-latitude", ["PSKY3"], lambda _: PHYS.replace('latitude="39.742476"', 'latitude="90.5"')),
    ("psky3-longitude", ["PSKY3"], lambda _: PHYS.replace('longitude="-105.1786"', 'longitude="-180.01"')),
    ("psky3-temperature", ["PSKY3"], lambda _: PHYS.replace('temperature="11"', 'temperature="-273"')),
    ("psky3-deltat", ["PSKY3"], lambda _: PHYS.replace('deltaT="67"', 'deltaT="8000.5"')),
    ("psky3-albedo", ["PSKY3"], lambda _: PHYS.replace(' sunSize="0.6"', ' sunSize="0.6" groundAlbedo="1.25"')),
    ("psky4-sun-elevation", ["PSKY4"], lambda _: PHYS.replace(' sunSize="0.6"', ' sunSize="0.6" sunElevation="30"')),
    ("physical-atmosphere-enum", ["S06"], lambda _: PHYS.replace(' sunSize="0.6"', ' sunSize="0.6" atmosphere="mars"')),
]
# the planets draft: primitive="planet" and its attributes, PL1 to PL5, V17 and the warning W19
PLANET = ('<scene version="1.8"><project width="64" height="64" fps="24" duration="1"/>'
    '<materials><material id="earth" baseColorMap="../media/logo.png" emissiveMap="../media/spin_0001.png"/><material id="photosphere" emissive="#FFF0E0"/></materials>'
    '<composition><camera id="eye" z="-10"/><object3D id="world" primitive="planet" body="399" material="earth" cloudMap="../media/spin_0002.png" cloudRotation="0.01"/></composition></scene>\n')
VALID["planet"] = PLANET
VALID["planet-knobs"] = PLANET.replace(' cloudRotation="0.01"', ' cloudRotation="-1.5e-2" cloudHeight="1" cloudShadow="false" nightLights="false" twilight="90" atmosphere="earth" atmosphereHeight="+11.1e3" ringMap="../media/spin_0003.png" ringInner="1.0001" ringOuter="+1.0002"')
VALID["planet-sun"] = PLANET.replace('body="399" material="earth"', 'body="10" material="photosphere" limbDarkening="solar"')
VALID["planet-radius"] = PLANET.replace('body="399"', 'body="2000001" radius="4" flattening="0.999"')
CASES += [
    ("v17-planet", ["V17"], lambda _: PLANET.replace('version="1.8"', 'version="1.7"')),
    ("pl1-body-without-radii", ["PL1"], lambda _: PLANET.replace('body="399"', 'body="401"')),
    ("pl2-sphere-clouds", ["PL2"], lambda _: PLANET.replace('primitive="planet" body="399"', 'primitive="sphere"')),
    ("pl3-flattening", ["PL3"], lambda _: PLANET.replace('body="399"', 'radius="4" flattening="1"')),
    ("pl3-twilight", ["PL3"], lambda _: PLANET.replace(' cloudRotation', ' twilight="90.5" cloudRotation')),
    ("pl3-cloud-height", ["PL3"], lambda _: PLANET.replace(' cloudRotation', ' cloudHeight="1.01" cloudRotation')),
    ("pl3-ring-inner", ["PL3"], lambda _: PLANET.replace(' cloudRotation', ' ringMap="../media/spin_0003.png" ringInner="1" cloudRotation')),
    ("pl3-ring-outer-default", ["PL3"], lambda _: PLANET.replace(' cloudRotation', ' ringMap="../media/spin_0003.png" ringInner="3" cloudRotation')),
    ("pl4-cloud-rotation", ["PL4"], lambda _: PLANET.replace(' cloudMap="../media/spin_0002.png"', '')),
    ("pl4-ring-outer", ["PL4"], lambda _: PLANET.replace(' cloudRotation', ' ringOuter="2" cloudRotation')),
    ("pl5-body", ["PL5"], lambda _: PLANET.replace('material="earth"', 'material="earth" limbDarkening="solar"')),
    ("pl5-material", ["PL5"], lambda _: PLANET.replace('body="399" material="earth"', 'body="10" limbDarkening="solar"')),
    ("planet-limb-darkening-enum", ["S06"], lambda _: PLANET.replace('material="earth"', 'material="earth" limbDarkening="quadratic"')),
    ("planet-atmosphere-enum", ["S06"], lambda _: PLANET.replace('material="earth"', 'material="earth" atmosphere="mars"')),
]
WARN_CASES += [
    # the planets draft: a body's radii override the flattening
    ("w19-planet-flattening", ["W19"], lambda _: PLANET.replace('body="399"', 'body="399" flattening="0.5"')),
]
CASES += [
    ("v16-fractal", ["V16"], lambda _: FRACTAL.replace('version="1.6"', 'version="1.5"')),
    ("frc1-julia-without-c", ["FRC1"], lambda _: FRACTAL.replace(' juliaX="-0.8" juliaY="0.156"', ' juliaX="-0.8"')),
    ("frc1-mandelbrot-with-c", ["FRC1"], lambda _: FRACTAL.replace('kind="mandelbrot"', 'kind="mandelbrot" juliaX="0" juliaY="0"')),
    ("s06-decimal-exponent", ["S06"], lambda _: FRACTAL.replace('centerY="+.131825904205311970493132"', 'centerY="1.3e-1"')),
]
# SREP 72: precision="f32" with 3D content is valid, and the evaluator adds the information finding PREC1 (the 3D
# renderer's internal buffers stay f16), which validate and the render report carry; sr-model's rules report nothing,
# because the 3D content is known only after expansion (crates/sr-eval/tests/srep72_precision.rs,
# crates/scene-render/tests/render_report.rs). The kit case is srep-0072-precision-3d-note.
VALID["prec1-f32-3d"] = ('<scene version="1.6"><project width="64" height="64" fps="24" duration="1" precision="f32"/>'
    '<composition><object3D id="ball" primitive="sphere" radius="20" x="32" y="32"/></composition></scene>\n')

# SREPs 76 and 80 (open issues): BH1 and VOX1 admit version 1.3 or later, so a black hole and cells can sit in a
# document of version 1.6 with the features that version gates; versions before 1.3 are still refused.
VALID["black-hole-version-1.6"] = BLACKHOLE.replace('version="1.3"', 'version="1.6"')
VALID["voxels-version-1.6"] = VOXELS.replace('version="1.3"', 'version="1.6"')
CASES += [
    ("bh1-version-1.0", ["BH1"], lambda _: BLACKHOLE.replace('version="1.3"', 'version="1.0"')),
]

# SREP 76, open issue 4: camera/@geodesics and camera/@denoise are xs:boolean, whose true is "true" or "1" and
# false "false" or "0", whitespace collapsed (XSD 1.0 Part 2, 3.2.2). The model reads every form, so BH5 to BH8 and
# the warnings W11, W12 and W05 test every form; the documents above write "true", these the other forms.
def geodesics(form):
    return lambda xml: xml.replace('geodesics="true"', f'geodesics="{form}"', 1)

VALID["black-hole-geodesics-1"] = geodesics("1")(BLACKHOLE)
VALID["black-hole-geodesics-padded"] = geodesics(" true ")(BLACKHOLE)
CASES += [
    ("bh5-geodesics-1-without-hole", ["BH5"], lambda _: geodesics("1")(BLACKHOLE.replace('<blackHole id="hole" mass="1" x="0" y="0" z="0"/>', '').replace('<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>', ''))),
    ("bh5-geodesics-padded-without-hole", ["BH5"], lambda _: geodesics(" 1 ")(BLACKHOLE.replace('<blackHole id="hole" mass="1" x="0" y="0" z="0"/>', '').replace('<accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>', ''))),
    ("bh6-geodesics-1-other-object", ["BH6"], lambda _: geodesics("1")(BLACKHOLE.replace('<blackHole', '<object3D id="ball" primitive="sphere" radius="1"/><blackHole'))),
    ("bh7-geodesics-1-inside-photon-sphere", ["BH7"], lambda _: geodesics("1")(BLACKHOLE.replace('z="-60" geodesics', 'z="-2.5" geodesics'))),
    # one camera writes "1", the other "true": both are geodesic cameras
    ("bh8-geodesics-1-and-true", ["BH8"], lambda _: geodesics("1")(BLACKHOLE).replace('<blackHole', '<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><blackHole')),
]
# SREP 77: crater/@mantle is an xs:boolean too; CRT11 and CRT12 read its forms "1" and "0" as the model does.
VALID["crater-mantle-1-bulking"] = CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="1" bulking="1.1"')
VALID["crater-mantle-0-repose"] = CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="0" repose="35"')
CASES += [
    ("crt11-bulking-with-mantle-0", ["CRT11"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="0" bulking="1.1"')),
    ("crt12-repose-with-mantle-1", ["CRT12"], lambda _: CRATER_IMPACT.replace('targetMaterial="softRock"', 'targetMaterial="softRock" mantle="1" repose="35"')),
]
WARN_CASES += [
    ("w11-geodesics-false", ["W11"], lambda _: geodesics("false")(BLACKHOLE)),
    ("w11-geodesics-0", ["W11"], lambda _: geodesics("0")(BLACKHOLE)),
    ("w12-denoise-1", ["W12"], lambda _: geodesics("1")(BLACKHOLE).replace('geodesics="1"', 'geodesics="1" denoise="1"')),
    ("w05-lights-geodesics-1", ["W05"], lambda _: geodesics("1")(BLACKHOLE).replace('</composition>', '</composition><lights><light id="sun" type="directional"/></lights>')),
]

# SREP 83: the gates of SREP 40 (V8, MSQ1, P3D1, OCN1, CRT1, FRX1, GEO1) refuse only 1.0 to 1.2, so a document of a
# later version keeps what SREP 40 adds, with the attributes of SREPs 77 to 80 on it; 1.2 is still refused
# (crt1, frx1, geo1, msq1, ocn1, p3d1, v8-volumes above).
for _name in ["volume", "mesh-sequence", "particles3d", "ocean", "crater", "fracture", "globe-relief", "crater-mantle",
              "fracture-contact", "ocean-density", "pyro-follow", "volume-scatter-bounces", "voxel-crater",
              "voxel-fracture"]:
    assert VALID[_name].count('<scene version="1.3"') == 1, _name
    VALID[f"{_name}-version-1.6"] = VALID[_name].replace('<scene version="1.3"', '<scene version="1.6"')
# a 1.6 feature (SREP 70's parametric surface, V14) beside 1.3 ones (SREP 40's crater and ocean) in one document
VALID["version-1.6-parametric-beside-crater"] = PARAMETRIC.replace(
    '</composition>', re.search(r'<composition>(.*)</composition>', CRATER, re.S).group(1).replace('id="ground"', 'id="pit"')
    + '<ocean id="sea" width="8" depth="8" cellSize="0.5"/></composition>')

# SREP 84: the rules read xs:boolean attributes by value, true for "true" and "1", false for "false" and "0",
# whitespace collapsed (XSD 1.0 Part 2, 3.2.2), and integer attributes by value, whatever their sign, leading zeros or
# padding (3.3.13), as the model does. Each document writes one attribute of C15, C43, R15, R48-from, R48-to, VOL5, C80,
# GEO2, PYRO9, PYRO10 or PYRO11 in a form other than the one the rule compared. c15-reverse-literal, which expected C15
# for reverse="0", is now c15-reverse-0 (valid); c15-reverse-1 keeps a refused reverse.
SPINNER = '<layer id="spinner" asset="spin">'
for _name, _attrs in [("c15-reverse-0", 'reverse="0"'), ("c15-reverse-padded-false", 'reverse=" false "'),
                      ("c15-loop-00", 'loop="00"'), ("c15-loop-plus-0", 'loop="+0"'), ("c15-loop-padded-0", 'loop=" 0 "')]:
    VALID[_name] = sub(SPINNER, f'<layer id="spinner" asset="spin" {_attrs}>')(base)
VALID["c43-alpha-0"] = sub('codec="h264" layout="square"', 'codec="h264" alpha="0" layout="square"')(base)
VALID["r15-has-audio-padded-true"] = sub('hasAudio="true"', 'hasAudio=" true "')(base)
VALID["r15-has-audio-padded-1"] = sub('hasAudio="true"', 'hasAudio=" 1 "')(base)
VALID["r48-threed-0"] = CONNECTOR.replace('<shape id="a" shape="rect"', '<shape id="a" threeD="0" shape="rect"')
VALID["vol5-blackbody-0"] = VOLUME.replace('<medium ', '<medium blackbody="0" ')
VALID["c80-from-plus-0"] = POINTS.replace('<repeat id="r"', '<repeat id="r" from="+0"')
VALID["c80-step-plus-1"] = POINTS.replace('<repeat id="r"', '<repeat id="r" step="+1"')
VALID["c80-step-01"] = POINTS.replace('<repeat id="r"', '<repeat id="r" step="01"')
VALID["geo2-tile-size-plus-64"] = GLOBE.replace('terrainTileSize="2"', 'terrainTileSize="+64"')
VALID["geo2-tile-size-064"] = GLOBE.replace('terrainTileSize="2"', 'terrainTileSize="064"')
VALID["pyro-follow-padded-1"] = PYRO_FOLLOW.replace('follow="true"', 'follow=" 1 "')
VALID["pyro-follow-margin-plus-2"] = PYRO_FOLLOW.replace('followMargin="2"', 'followMargin="+2"')
CASES += [
    ("c15-reverse-padded-true", ["C15"], sub(SPINNER, '<layer id="spinner" asset="spin" reverse=" true ">')),
    ("c15-loop-plus-2", ["C15"], sub(SPINNER, '<layer id="spinner" asset="spin" loop="+2">')),
    ("c43-alpha-1", ["C43"], sub('codec="h264" layout="square"', 'codec="h264" alpha="1" layout="square"')),
    ("c43-alpha-padded-true", ["C43"], sub('codec="h264" layout="square"', 'codec="h264" alpha=" true " layout="square"')),
    ("r15-has-audio-0", ["R15"], sub('hasAudio="true"', 'hasAudio="0"')),
    ("r48-from-threed-1", ["R48-from"], lambda _: CONNECTOR.replace('<shape id="a" shape="rect"', '<shape id="a" threeD="1" shape="rect"')),
    ("r48-from-threed-padded-true", ["R48-from"], lambda _: CONNECTOR.replace('<shape id="a" shape="rect"', '<shape id="a" threeD=" true " shape="rect"')),
    ("r48-to-threed-1", ["R48-to"], lambda _: CONNECTOR.replace('<shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/>', '<group id="g" threeD="1"><shape id="b" shape="rect" x="44" y="40" width="10" height="10" fill="#0000FF"/></group>')),
    ("vol5-blackbody-padded-true", ["VOL5"], lambda _: VOLUME.replace('<medium ', '<medium blackbody=" true " ')),
    ("vol5-blackbody-padded-1", ["VOL5"], lambda _: VOLUME.replace('<medium ', '<medium blackbody=" 1 " ')),
    ("c80-from-plus-2", ["C80"], lambda _: POINTS.replace('<repeat id="r"', '<repeat id="r" from="+2"')),
    ("geo2-tile-size-plus-48", ["GEO2"], lambda _: GLOBE.replace('terrainTileSize="2"', 'terrainTileSize="+48"')),
    ("pyro9-follow-padded-true-closed", ["PYRO9"], lambda _: PYRO_FOLLOW.replace('boundary="open" ', '').replace('follow="true"', 'follow=" true "')),
    ("pyro10-follow-0", ["PYRO10"], lambda _: PYRO_FOLLOW.replace('follow="true"', 'follow="0"')),
    ("pyro11-margin-plus-4", ["PYRO11"], lambda _: PYRO_FOLLOW.replace('followMargin="2"', 'followMargin="+4"')),
]

# SREP 85: the rules read xs:double and xs:decimal attributes (and SREP 67's ITR1 integers) by value, whatever their
# sign: "+1" and "+0.5" are 1 and 0.5 (XSD 1.0 Part 2, 3.2.3.1, 3.2.5.1), as the model reads them, where XPath 1.0's
# number() and sum() read NaN. Each valid document writes attributes of C13, C14, C15, C44, C52, C54, C56, PYRO1, PYRO2,
# VOL4, P3D3, P3D10, OCN3, PYRO11, PYC6, FRX11, ITR1, SRT1 or OCN14 with a '+'; each invalid one writes, with a '+', what
# C37, C38, C53, PYRO4, OCN3 (wavelength) or OCN5 forbids, which a NaN let through. c15-speed-exponent writes speed="1e0",
# for which the native validator is normative (SREP 85, Semantics 4; libxml2 reads it too).
VALID["c13-min-size-plus-0.5"] = sub('minSize="24" maxSize="96"', 'minSize="+0.5" maxSize="+96"')(base)
VALID["c14-letter-spacing-plus-0.5"] = sub('letterSpacing="2"', 'letterSpacing="+0.5"')(base)
VALID["c15-speed-plus-1"] = sub(SPINNER, '<layer id="spinner" asset="spin" speed="+1">')(base)
VALID["c15-speed-exponent"] = sub(SPINNER, '<layer id="spinner" asset="spin" speed="1e0">')(base)
VALID["c44-start-plus-0.5"] = sub('start="0" end="12"', 'start="+0.5" end="+12"')(base)
VALID["c52-end-plus-1"] = ins_comp('<fluid id="fl" width="10" height="10"><fluidSource start="+0.5" end="+1"/></fluid>')(base)
VALID["c54-start-plus-0"] = sub('<output id="short" start="0" path', '<output id="short" start="+0" path')(base)
VALID["c56-from-plus-0.5"] = sub('<segment from="0" to="4"', '<segment from="+0.5" to="+4"')(base)
VALID["pyro1-voxel-size-plus-0.5"] = PYRO.replace('voxelSize="1"', 'voxelSize="+0.5"')
VALID["pyro2-end-plus-1"] = PYRO.replace('end="0.35"', 'end="+1"')
VALID["vol4-bounds-max-z-plus-1"] = VOLUME.replace('boundsMaxZ="2"', 'boundsMaxZ="+1"')
VALID["p3d3-lifetime-plus-1"] = PARTICLES3D.replace('lifetime="2"', 'lifetime="+1" lifetimeVariance="+0.5"')
VALID["p3d10-angle-plus-1"] = EJECTA_CRATER.replace('count="200"/>', 'count="200" angle="+1" angleSpread="+0.5"/>')
VALID["ocn3-cell-size-plus-0.5"] = OCEAN.replace('cellSize="0.5"', 'cellSize="+0.5"')
VALID["ocn14-foam-radius-plus-1"] = OCEAN.replace('</ocean>', '<whitewater foamRadius="+1"/></ocean>')
VALID["pyro11-voxel-size-plus-1"] = PYRO_FOLLOW.replace('voxelSize="1"', 'voxelSize="+1"')
VALID["pyc6-x-plus-1"] = PYRO_BLAST.replace('x="1"', 'x="+1"')
VALID["frx11-planes-plus-1"] = VOXEL_FRACTURE.replace('pieces="8" seed="3"', 'partition="planes" planes="+1 0 0 +0.5"')
VALID["itr1-check-every-plus-1"] = SREP67.format(body=f'<iterate id="it" steps="10" checkEvery="+1">{SREP67_SHAPE}</iterate>', post='<effects><effect id="fx" type="blur"/></effects>')
VALID["srt1-low-plus-0.5"] = SREP67.format(body=SREP67_SHAPE, post='<effects><effect id="fx" type="segmented-sort" low="+0.5" high="+1"/></effects>')
CASES += [
    ("c37-offset-plus-0.2-after-0.5", ["C37"], sub('<stop offset="0.5" color="var(--brand)"/>', '<stop offset="0.5" color="var(--brand)"/>\n      <stop offset="+0.2" color="#000000"/>')),
    ("c38-time-plus-0.25-after-0.5", ["C38"], sub('<key time="1" value="0.4"/>', '<key time="0.5" value="0.4"/>\n          <key time="+0.25" value="0.5"/>')),
    ("c53-domain-plus-3-then-plus-1", ["C53"], sub('palette="#F7FBFF #08306B"', 'palette="#F7FBFF #08306B" domain="+3 +1"')),
    ("pyro4-scale-x-plus-0", ["PYRO4"], lambda _: PYRO.replace('radius="2"', 'radius="2" scaleX="+0"')),
    ("ocn3-wavelength-plus-0.5", ["OCN3"], lambda _: OCEAN.replace('wavelength="4"', 'wavelength="+0.5"')),
    ("ocn5-end-plus-0.5-before-start-plus-1", ["OCN5"], lambda _: OCEAN.replace('</ocean>', '<whitewater start="+1" end="+0.5"/></ocean>')),
    ("srt1-low-plus-1-above-high-plus-0.5", ["SRT1"], lambda _: SREP67.format(body=SREP67_SHAPE, post='<effects><effect id="fx" type="segmented-sort" low="+1" high="+0.5"/></effects>')),
]

# The special values of xs:double: INF, -INF and NaN are NaN to XPath's number(), so each rule compares them as the
# Schematron writes the comparison (SREP 85 leaves them as before): OCN3's not(wave[wl < 2*$cell]) and OCN5's
# not(whitewater[@end and end < start]) let a NaN through, P3D3 has no finiteness clause on the direction (NaN != 0),
# and OCN7/P3D6/PYRO8 read clay's boil with number(translate(@boil,'+','')), so boil="INF" does not animate the
# collider. These documents are valid with W01 (the renderer needs finite values and refuses them when it runs).
# PYRO2's end="INF" is NaN there too, so the source is refused, by the Schematron and by the native rules.
WARN_CASES += [
    ("ocn3-wavelength-inf", ["W01"], lambda _: OCEAN.replace('wavelength="4"', 'wavelength="INF"')),
    ("ocn5-whitewater-end-inf", ["W01"], lambda _: OCEAN.replace('</ocean>', '<whitewater start="0" end="INF"/></ocean>')),
    ("ocn5-whitewater-start-inf", ["W01"], lambda _: OCEAN.replace('</ocean>', '<whitewater start="INF" end="1"/></ocean>')),
    ("p3d3-direction-x-inf", ["W01"], lambda _: PARTICLES3D.replace('<particles3D id="dust" ', '<particles3D id="dust" directionX="INF" ')),
    ("solid-colliders-boil-inf", ["W01"], lambda _: SOLID_COLLIDERS.replace('primitive="clay"', 'primitive="clay" boil="INF" fingerprints="1"')),
]
CASES += [
    ("pyro2-end-inf", ["PYRO2", "W01"], lambda _: PYRO.replace('end="0.35"', 'end="INF"')),
    # VOL5 accepts any pyro child ("or pyro"); two of them fail VOL1 only
    ("vol5-two-pyros", ["VOL1"], lambda _: PYRO.replace('</pyro><medium', '</pyro><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"/><medium')),
]
# sr-core's OCN7 leaves 'tracking' out of a text collider's shape parameters, which its P3D6 and PYRO8 name (SREP 37:
# tracking changes the collider). The ocean builds its colliders' geometry once, so this engine refuses an animated
# tracking on an ocean collider under OCN7, as it does for particles and pyro; the oracle accepts the document.
RULE_CASES += [
    ("ocn7-text-tracking", ["OCN7"], lambda _: OCEAN.replace('<composition>', '<composition><object3D id="letters" primitive="text" text="O"><animate property="tracking"><key time="0" value="10"/></animate></object3D>').replace('<ocean id="sea" ', '<ocean id="sea" colliders="letters" ')),
]

def write(path, text, expect):
    header = f"<!-- expect: {' '.join(expect) if expect else 'valid'} -->\n"
    body = text.split("\n", 1)[1] if text.startswith("<?xml") else text
    decl = '<?xml version="1.0" encoding="UTF-8"?>\n'
    path.write_text(decl + header + body)
    return (decl + header + body).encode()

def small_cells(text):
    """Whether a voxels object of the document has cells smaller than 0.5 in the scene (rules.rs W09): its cellSize, else its asset's, times the smallest scale."""
    import xml.etree.ElementTree as ET
    try:
        root = ET.fromstring(re.sub(r"<!--.*?-->", "", text, flags=re.S))
    except ET.ParseError:
        return False
    assets = {a.get("id"): a for a in root.iter() if a.tag.endswith("voxelAsset")}
    for o in root.iter():
        if not o.tag.endswith("object3D") or o.get("primitive") != "voxels":
            continue
        size = o.get("cellSize") or (assets[o.get("voxels")].get("cellSize") if o.get("voxels") in assets else None)
        try:
            cell = float(size.strip())
            scale = min(abs(float(o.get(k, "1").strip())) for k in ("scaleX", "scaleY", "scaleZ"))
        except (AttributeError, ValueError):
            continue
        if cell * scale < 0.5:
            return True
    return False


def with_small_cells(text, expect):
    """The expected codes of a document, with W09 when it has cells smaller than 0.5 in the scene."""
    return list(expect) + ["W09"] if small_cells(text) and "W09" not in expect else list(expect)


def oracle_codes(expected, blind=False):
    """Translate Rust diagnostics into the independent schema oracle's scope."""
    # structural (S01...), asset (A01...) and warning (W01...) codes are a letter and digits; a Schematron id may start
    # with one of those letters too (SRT1 and STP1 of SREPs 67 and 68)
    outside = re.compile(r"[SAW]\d")
    codes = {code for code in expected if not outside.match(code)}
    if not blind and any(re.match(r"S\d", code) for code in expected):
        codes.add("XSD")
    return sorted(codes)


def main():
    manifest = {"valid": {}, "invalid": {}, "warnings": {}}
    for d in ("valid", "invalid"):
        for f in (CORPUS / d).glob("*.xml"):
            f.unlink()
    ok = True
    for name, text in VALID.items():
        expect = with_small_cells(text, [])
        data = write(CORPUS / "valid" / f"{name}.scene.xml", text, expect)
        xsd, sch = verdict(data)
        if xsd or sch:
            ok = False
            print(f"valid/{name}: oracle disagrees: {xsd} {sch}")
        manifest["warnings" if expect else "valid"][f"{name}.scene.xml"] = expect
    for name, expect, fn in CASES:
        text = fn(base)
        expect = with_small_cells(text, expect)
        data = write(CORPUS / "invalid" / f"{name}.scene.xml", text, expect)
        xsd, sch = verdict(data)
        got = sorted(set(i for i, _ in sch) | ({"XSD"} if xsd else set()))
        # Asset diagnostics and renderer warnings are outside XSD/Schematron.
        want = oracle_codes(expect, blind=name in ORACLE_BLIND)
        if got != want:
            ok = False
            print(f"invalid/{name}: expected {want}, oracle says {got}: {xsd[:2]}")
        manifest["invalid"][f"{name}.scene.xml"] = expect
    for name, expect, fn in ASSET_CASES + RULE_CASES + WARN_CASES:
        is_warn = (name, expect, fn) in WARN_CASES
        text = fn(base)
        expect = with_small_cells(text, expect)
        data = write(CORPUS / ("valid" if is_warn else "invalid") / f"{name}.scene.xml", text, expect)
        xsd, sch = verdict(data)
        if xsd or sch:
            ok = False
            print(f"{name}: oracle should accept: {xsd} {sch}")
        manifest["warnings" if is_warn else "invalid"][f"{name}.scene.xml"] = expect
    (CORPUS / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    n = sum(len(v) for v in manifest.values())
    print(f"{n} documents written; oracle agreement: {'yes' if ok else 'NO'}")
    return 0 if ok else 1

if __name__ == "__main__":
    sys.exit(main())
