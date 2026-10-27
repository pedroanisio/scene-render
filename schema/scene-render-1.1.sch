<?xml version="1.0" encoding="UTF-8"?>
<!-- scene-render 1.1 cross-field rules. Run after XSD validation.
     ISO Schematron, XPath 1.0 + EXSLT strings (supported by libxslt / lxml / Saxon). -->
<sch:schema xmlns:sch="http://purl.oclc.org/dsdl/schematron" queryBinding="xslt">
  <sch:title>scene-render 1.1 co-occurrence and reference rules</sch:title>
  <sch:ns prefix="str" uri="http://exslt.org/strings"/>
  <sch:ns prefix="math" uri="http://exslt.org/math"/>

  <!-- ============================================================ version gate -->
    <sch:pattern id="p1">
    <sch:rule context="/scene[@version='1.0']">
      <sch:assert id="V1" test="not(metadata|parameters|styles|colorManagement|layouts|safeAreas|paints|symbols|markers|tracking|captions)">
        version="1.0" documents cannot use 1.1 sections; set version="1.1".</sch:assert>
      <sch:assert id="V2" test="count(output) &lt;= 1">version="1.0" allows one output element.</sch:assert>
      <sch:assert id="V3" test="not(.//sequence|.//instance|.//include|.//repeat|.//adjustment|.//transition|.//skeleton|.//expression|.//motionPath|.//link|.//timeRemap|.//textAnimator|.//textPath|.//shapeModifier|.//transformConstraint)">
        version="1.0" documents cannot use 1.1 node or animation elements.</sch:assert>
      <sch:assert id="V4" test="not(assets/imageSequence|assets/lottie|assets/font|assets/generator|assets/chart|assets/audiogram|assets/code|assets/formula|assets/generated)">
        version="1.0" documents cannot use 1.1 asset kinds.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p1b">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V5" test="not(assets/tiles|.//basemap|.//object3D/rigidBody|.//object3D/morph|.//object3D/joint|assets/strokeFont|.//shape[@shape='stroke-text']|.//object3D[@primitive='map' or @primitive='globe']|output/segment|output/audioTrack|output/captionTrack)">
        documents before version="1.2" cannot use 1.2 elements or asset kinds; set version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p2">
    <sch:rule context="vector">
      <sch:assert id="C1" test="not(@shape='path') or @path">vector shape="path" requires @path.</sch:assert>
      <sch:assert id="C2" test="not(@shape='svg') or @src">vector shape="svg" requires @src.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p3">
    <sch:rule context="shape">
      <sch:assert id="C3" test="not(@shape='path') or @path">shape shape="path" requires @path.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p4">
    <sch:rule context="mask">
      <sch:assert id="C4" test="not(@type='path') or @path">mask type="path" requires @path.</sch:assert>
      <sch:assert id="C5" test="@type='path' or (@width and @height)">mask type="<sch:value-of select="@type"/>" requires @width and @height.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p5">
    <sch:rule context="object3D">
      <sch:assert id="C6" test="not(@primitive='mesh') or @mesh">object3D primitive="mesh" requires @mesh.</sch:assert>
      <sch:assert id="C7" test="not(@primitive='text') or @text">object3D primitive="text" requires @text.</sch:assert>
      <sch:assert id="C8" test="not(@primitive='extrude') or @path">object3D primitive="extrude" requires @path.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p6">
    <sch:rule context="constraint[@type='pin']">
      <sch:assert id="C9" test="not(@b)">pin constraints tie body a to a world point; @b is not allowed.</sch:assert>
      <sch:assert id="C10" test="@x and @y">pin constraints require @x and @y.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p7">
    <sch:rule context="constraint[@type!='pin']">
      <sch:assert id="C11" test="@b">constraint type="<sch:value-of select="@type"/>" requires @b.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p8">
    <sch:rule context="assets/text">
      <sch:assert id="C12" test="(@text and not(span)) or (not(@text) and span)">text asset "<sch:value-of select="@id"/>" needs exactly one of @text or span children.</sch:assert>
      <sch:assert id="C13" test="not(@minSize and @maxSize) or number(substring(normalize-space(@minSize),1+number(starts-with(normalize-space(@minSize),'+')))) &lt;= number(substring(normalize-space(@maxSize),1+number(starts-with(normalize-space(@maxSize),'+'))))">minSize must not exceed maxSize.</sch:assert>
      <sch:assert id="C14" test="not(@letterSpacing) or (number(substring(normalize-space(@letterSpacing),1+number(starts-with(normalize-space(@letterSpacing),'+')))) &gt;= -number(substring(normalize-space(@size),1+number(starts-with(normalize-space(@size),'+')))) and number(substring(normalize-space(@letterSpacing),1+number(starts-with(normalize-space(@letterSpacing),'+')))) &lt;= 4 * number(substring(normalize-space(@size),1+number(starts-with(normalize-space(@size),'+')))))">letterSpacing must lie in [-size, 4 x size].</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p9">
    <sch:rule context="layer[timeRemap]">
      <sch:assert id="C15" test="(not(@speed) or number(substring(normalize-space(@speed),1+number(starts-with(normalize-space(@speed),'+'))))=1) and (not(@reverse) or normalize-space(@reverse)='false' or normalize-space(@reverse)='0') and (not(@loop) or number(substring(normalize-space(@loop),1+number(starts-with(normalize-space(@loop),'+'))))=0)">
        layer "<sch:value-of select="@id"/>": timeRemap replaces speed, reverse and loop; remove them.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p10">
    <sch:rule context="layer[@fit and @fit!='none'] | instance[@fit and @fit!='none']">
      <sch:assert id="C16" test="@boxWidth and @boxHeight">fit="<sch:value-of select="@fit"/>" requires @boxWidth and @boxHeight.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p11">
    <sch:rule context="repeat">
      <sch:assert id="C17" test="count(@count) + count(@over) + number(boolean(points)) = 1">repeat needs exactly one of @count, @over or a points child.</sch:assert>
      <sch:assert id="C18" test="not(@over) or /scene/parameters/data[@id=current()/@over] or /scene/parameters/param[@id=current()/@over][@type='list'] or /scene/parameters/program[@id=current()/@over]">repeat/@over must name a data source, a list parameter or a data program (SREP 66).</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p12">
    <sch:rule context="effect[@type='lut' or @type='shader' or @type='displacement-map' or @type='gradient-map']">
      <sch:assert id="C19" test="@src or @source or @paint">effect type="<sch:value-of select="@type"/>" requires @src, @source or @paint.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p13">
    <sch:rule context="transition">
      <sch:assert id="C20" test="@from or @to or parent::segment">transition needs @from, @to or both.</sch:assert>
      <sch:assert id="C21" test="not(@type='shader') or @shader">transition type="shader" requires @shader.</sch:assert>
      <sch:assert id="C22" test="not(@type='luma') or @matte">transition type="luma" requires @matte.</sch:assert>
      <sch:assert id="C23" test="not(@from) or count(../*[@id=current()/@from])=1">transition/@from must be a sibling of the transition.</sch:assert>
      <sch:assert id="C24" test="not(@to) or count(../*[@id=current()/@to])=1">transition/@to must be a sibling of the transition.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p14">
    <sch:rule context="modifier">
      <sch:assert id="C25" test="not(@type='skin') or @skeleton">modifier type="skin" requires @skeleton.</sch:assert>
      <sch:assert id="C26" test="not(@type='corner-pin') or count(str:tokenize(normalize-space(@corners),' '))=8">modifier type="corner-pin" requires 8 numbers in @corners.</sch:assert>
      <sch:assert id="SPT1" test="(not(@type='map') or (@x and @y)) and (not(@type='complex') or @function) and (not(@function='mobius') or (@coefficients and count(str:tokenize(normalize-space(@coefficients), ' '))=8)) and (not(@coefficients) or @function='mobius') and (@type='map' or not(@x or @y)) and (@type='complex' or not(@function or @coefficients))">a map modifier has x and y, a complex one a function, a mobius eight coefficients; no other modifier has them.</sch:assert>
      <sch:assert id="SPT2" test="not(@function='mobius') or count(str:tokenize(normalize-space(@coefficients), ' ')) != 8 or number(str:tokenize(normalize-space(@coefficients), ' ')[1]) * number(str:tokenize(normalize-space(@coefficients), ' ')[7]) - number(str:tokenize(normalize-space(@coefficients), ' ')[2]) * number(str:tokenize(normalize-space(@coefficients), ' ')[8]) - number(str:tokenize(normalize-space(@coefficients), ' ')[3]) * number(str:tokenize(normalize-space(@coefficients), ' ')[5]) + number(str:tokenize(normalize-space(@coefficients), ' ')[4]) * number(str:tokenize(normalize-space(@coefficients), ' ')[6]) != 0 or number(str:tokenize(normalize-space(@coefficients), ' ')[1]) * number(str:tokenize(normalize-space(@coefficients), ' ')[8]) + number(str:tokenize(normalize-space(@coefficients), ' ')[2]) * number(str:tokenize(normalize-space(@coefficients), ' ')[7]) - number(str:tokenize(normalize-space(@coefficients), ' ')[3]) * number(str:tokenize(normalize-space(@coefficients), ' ')[6]) - number(str:tokenize(normalize-space(@coefficients), ' ')[4]) * number(str:tokenize(normalize-space(@coefficients), ' ')[5]) != 0">a mobius modifier's coefficients have a d - b c != 0.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p15">
    <sch:rule context="transformConstraint">
      <sch:assert id="C27" test="not(@type='follow-path') or @path">follow-path requires @path.</sch:assert>
      <sch:assert id="C28" test="@type='follow-path' or @target">transformConstraint type="<sch:value-of select="@type"/>" requires @target.</sch:assert>
      <sch:assert id="C29" test="not(@type='track') or /scene/tracking/trackData[@id=current()/@target]">track constraints target trackData.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p16">
    <sch:rule context="safeArea[@preset='custom' or not(@preset)]">
      <sch:assert id="C30" test="@top and @right and @bottom and @left">custom safe areas require top, right, bottom and left.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p17">
    <sch:rule context="captionTrack">
      <sch:assert id="C31" test="count(cue[1]|@src|@transcribe)=1">captionTrack "<sch:value-of select="@id"/>" needs exactly one source: cue children, @src or @transcribe.</sch:assert>
      <sch:assert id="C32" test="not(@transcribe) or (@cache and @cacheSha256)">transcribed captions require @cache and @cacheSha256 (deterministic renders).</sch:assert>
      <sch:assert id="C33" test="not(@transcribe) or parent::output or /scene/audioMix/audioTrack[@id=current()/@transcribe]">captionTrack/@transcribe must name an audioTrack.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p18">
    <sch:rule context="audioMix">
      <sch:assert id="C34" test="count(master) &lt;= 1">audioMix allows at most one master.</sch:assert>
      <sch:assert id="C35" test="audioTrack">audioMix needs at least one audioTrack.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p19">
    <sch:rule context="linearGradient|radialGradient|conicGradient">
      <sch:assert id="C36" test="count(stop) &gt;= 2">gradient "<sch:value-of select="@id"/>" needs at least two stops.</sch:assert>
      <sch:assert id="C37" test="not(stop[number(substring(normalize-space(@offset),1+number(starts-with(normalize-space(@offset),'+')))) &lt; number(substring(normalize-space(preceding-sibling::stop[1]/@offset),1+number(starts-with(normalize-space(preceding-sibling::stop[1]/@offset),'+'))))])">gradient stop offsets must be non-decreasing.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p20">
    <sch:rule context="animate|timeRemap">
      <sch:assert id="C38" test="not(key[number(substring(normalize-space(@time),1+number(starts-with(normalize-space(@time),'+')))) &lt; number(substring(normalize-space(preceding-sibling::key[1]/@time),1+number(starts-with(normalize-space(preceding-sibling::key[1]/@time),'+'))))])">key times must be non-decreasing.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p21">
    <sch:rule context="key[@interpolation='steps']">
      <sch:assert id="C39" test="@steps">interpolation="steps" requires @steps.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p22">
    <sch:rule context="key[@interpolation='cubic-bezier']">
      <sch:assert id="C40" test="@bezier or @easeOut or following-sibling::key[1]/@easeIn">cubic-bezier keys need @bezier or easeOut/easeIn handles.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p23">
    <sch:rule context="generated">
      <sch:assert id="C41" test="@prompt or @kind='speech'">generated media other than speech requires @prompt.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p24">
    <sch:rule context="output">
      <sch:assert id="C42" test="not(@proresProfile) or @codec='prores'">proresProfile applies only to codec="prores".</sch:assert>
      <sch:assert id="C43" test="not(normalize-space(@alpha)='true' or normalize-space(@alpha)='1') or @codec='prores' or @codec='vp9' or @codec='ffv1' or @codec='png-sequence' or @codec='exr-sequence' or @codec='tiff-sequence' or @codec='apng' or @codec='webp' or @codec='gif'">alpha="true" needs a codec that carries alpha.</sch:assert>
      <sch:assert id="C44" test="not(@end) or (not(@start) and number(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+')))) &gt; 0) or number(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+')))) &gt; number(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+'))))">output end must be after start.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p25">
    <sch:rule context="layer">
      <sch:assert id="R1" test="/scene/assets/*[@id=current()/@asset]">layer "<sch:value-of select="@id"/>": @asset must reference an element of assets.</sch:assert>
      <sch:assert id="R2" test="not(textAnimator or textPath) or /scene/assets/text[@id=current()/@asset]">textAnimator and textPath need a text asset.</sch:assert>
      <sch:assert id="FRM1" test="not(formulaAnimator) or /scene/assets/formula[@id=current()/@asset]">formulaAnimator needs a formula asset.</sch:assert>
      <sch:assert id="R3" test="not(@audioBus) or /scene/audioMix/bus[@id=current()/@audioBus]">layer/@audioBus must name a bus.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p26">
    <sch:rule context="object3D">
      <sch:assert id="C47" test="not(@primitive='map' or @primitive='globe') or @map">object3D primitive="map" or "globe" requires @map.</sch:assert>
      <sch:assert id="R28" test="not(@map) or /scene/assets/map[@id=current()/@map]">object3D/@map must name a map asset.</sch:assert>
      <sch:assert id="R29" test="not(@terrain) or /scene/assets/tiles[@id=current()/@terrain]">object3D/@terrain must name a tiles asset.</sch:assert>
      <sch:assert id="R4" test="not(@material) or /scene/materials/material[@id=current()/@material]">object3D/@material must name a material.</sch:assert>
      <sch:assert id="R5" test="not(@mesh) or /scene/assets/*[(self::mesh or self::meshSequence) and @id=current()/@mesh]">object3D/@mesh must name a mesh asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p27">
    <sch:rule context="*[@effects]">
      <sch:assert id="R6" test="count(str:tokenize(normalize-space(@effects),' ')) = count(/scene/effects/effect[contains(concat(' ',normalize-space(current()/@effects),' '), concat(' ',@id,' '))])">
        every id in @effects of "<sch:value-of select="@id"/>" must name an effect.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p28">
    <sch:rule context="effect[@lights]">
      <sch:assert id="R7" test="count(str:tokenize(normalize-space(@lights),' ')) = count(/scene/lights/light[contains(concat(' ',normalize-space(current()/@lights),' '), concat(' ',@id,' '))])">every id in effect/@lights must name a light.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p29">
    <sch:rule context="*[@matte][not(self::transition)]">
      <sch:assert id="R8" test="/scene/composition//*[@id=current()/@matte] or /scene/symbols//*[@id=current()/@matte]">@matte must name a composition node.</sch:assert>
      <sch:assert id="R9" test="@matte != @id">a node cannot be its own matte.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p30">
    <sch:rule context="*[@parent][@id]">
      <sch:assert id="R10" test="@parent != @id">a node cannot parent itself.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p31">
    <sch:rule context="instance">
      <sch:assert id="R11" test="/scene/symbols/symbol[@id=current()/@symbol]">instance/@symbol must name a symbol.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p32">
    <sch:rule context="output">
      <sch:assert id="R12" test="not(@layout) or /scene/layouts/layout[@id=current()/@layout]">output/@layout must name a layout.</sch:assert>
      <sch:assert id="R13" test="not(@variant) or /scene/parameters/variant[@id=current()/@variant]">output/@variant must name a variant.</sch:assert>
      <sch:assert id="R14" test="not(@burnCaptions) or /scene/captions/captionTrack[@id=current()/@burnCaptions] or captionTrack[@id=current()/@burnCaptions]">output/@burnCaptions must name a captionTrack.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p33">
    <sch:rule context="audioTrack">
      <sch:assert id="R15" test="/scene/assets/audio[@id=current()/@asset] or /scene/assets/generated[@id=current()/@asset][@kind='speech' or @kind='music' or @kind='sound-effect'] or /scene/assets/video[@id=current()/@asset][normalize-space(@hasAudio)='true' or normalize-space(@hasAudio)='1']">audioTrack/@asset must name audio, generated audio, or video with hasAudio="true".</sch:assert>
      <sch:assert id="R16" test="not(@bus) or /scene/audioMix/bus[@id=current()/@bus]">audioTrack/@bus must name a bus.</sch:assert>
      <sch:assert id="R17" test="not(@duckUnder) or not(contains(concat(' ',normalize-space(@duckUnder),' '), concat(' ',@id,' ')))">a track cannot duck under itself.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p34">
    <sch:rule context="bind">
      <sch:assert id="R18" test="/scene/parameters/param[@id=current()/@param]">bind/@param must name a param.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p35">
    <sch:rule context="project[@safeArea] | layout[@safeArea] | captionTrack[@safeArea]">
      <sch:assert id="R19" test="/scene/safeAreas/safeArea[@id=current()/@safeArea]">@safeArea must name a safeArea.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p36">
    <sch:rule context="camera[@focusTarget] | camera[@target]">
      <sch:assert id="R20" test="(not(@focusTarget) or /scene/composition//*[@id=current()/@focusTarget]) and (not(@target) or /scene/composition//*[@id=current()/@target])">camera targets must name composition nodes.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p37">
    <sch:rule context="*[@startMarker] | *[@endMarker] | key[@marker]">
      <sch:assert id="R21" test="(not(@startMarker) or /scene/markers/marker[@id=current()/@startMarker] or (/scene/markers/beatGrid and ((substring-before(@startMarker,'.')='beat' or substring-before(@startMarker,'.')='bar') and string-length(substring-after(@startMarker,'.')) &gt; 0 and string-length(substring-after(@startMarker,'.')) &lt; 20 and translate(substring-after(@startMarker,'.'),'0123456789','')=''))) and (not(@endMarker) or /scene/markers/marker[@id=current()/@endMarker] or (/scene/markers/beatGrid and ((substring-before(@endMarker,'.')='beat' or substring-before(@endMarker,'.')='bar') and string-length(substring-after(@endMarker,'.')) &gt; 0 and string-length(substring-after(@endMarker,'.')) &lt; 20 and translate(substring-after(@endMarker,'.'),'0123456789','')=''))) and (not(@marker) or /scene/markers/marker[@id=current()/@marker] or (/scene/markers/beatGrid and ((substring-before(@marker,'.')='beat' or substring-before(@marker,'.')='bar') and string-length(substring-after(@marker,'.')) &gt; 0 and string-length(substring-after(@marker,'.')) &lt; 20 and translate(substring-after(@marker,'.'),'0123456789','')='')))">marker references must name a marker, or a beat.N or bar.N id of the scene's beatGrid.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p38">
    <sch:rule context="textStyle[@basedOn] | *[@style][not(self::audiogram)]">
      <sch:assert id="R22" test="(not(@basedOn) or /scene/styles/textStyle[@id=current()/@basedOn]) and (not(@style) or /scene/styles/textStyle[@id=current()/@style])">text style references must name textStyle elements.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p39">
    <sch:rule context="*[@fontAsset]">
      <sch:assert id="R23" test="/scene/assets/font[@id=current()/@fontAsset]">@fontAsset must name a font asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p40">
        <sch:rule context="*[@fill or @stroke or @color or @background or @paint or @activeColor or @highlight or @strokeColor]">
      <sch:assert id="R24-fill" test="not(starts-with(@fill,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@fill,'url(#'),')')]">@fill: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-stroke" test="not(starts-with(@stroke,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@stroke,'url(#'),')')]">@stroke: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-color" test="not(starts-with(@color,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@color,'url(#'),')')]">@color: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-background" test="not(starts-with(@background,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@background,'url(#'),')')]">@background: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-paint" test="not(starts-with(@paint,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@paint,'url(#'),')')]">@paint: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-activeColor" test="not(starts-with(@activeColor,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@activeColor,'url(#'),')')]">@activeColor: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-highlight" test="not(starts-with(@highlight,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@highlight,'url(#'),')')]">@highlight: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R24-strokeColor" test="not(starts-with(@strokeColor,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@strokeColor,'url(#'),')')]">@strokeColor: url(#id) must name an element of paints.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p41">
    <sch:rule context="*[@fill or @stroke or @color or @background or @paint or @activeColor or @highlight or @strokeColor]">
      <sch:assert id="R25-fill" test="not(starts-with(@fill,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@fill,'var(--'),')')]">@fill: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-stroke" test="not(starts-with(@stroke,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@stroke,'var(--'),')')]">@stroke: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-color" test="not(starts-with(@color,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@color,'var(--'),')')]">@color: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-background" test="not(starts-with(@background,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@background,'var(--'),')')]">@background: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-paint" test="not(starts-with(@paint,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@paint,'var(--'),')')]">@paint: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-activeColor" test="not(starts-with(@activeColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@activeColor,'var(--'),')')]">@activeColor: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-highlight" test="not(starts-with(@highlight,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@highlight,'var(--'),')')]">@highlight: var(--name) must name a style token.</sch:assert>
      <sch:assert id="R25-strokeColor" test="not(starts-with(@strokeColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@strokeColor,'var(--'),')')]">@strokeColor: var(--name) must name a style token.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p42">
    <sch:rule context="geoLayer">
      <sch:assert id="R36" test="/scene/assets/geo[@id=current()/@geo]">geoLayer/@geo must name a geo asset.</sch:assert>
    </sch:rule>
    <sch:rule context="route">
      <sch:assert id="R37" test="not(@geo) or /scene/assets/geo[@id=current()/@geo]">route/@geo must name a geo asset.</sch:assert>
      <sch:assert id="C45" test="@points or @geo">route needs @points or @geo.</sch:assert>
    </sch:rule>
    <sch:rule context="basemap">
      <sch:assert id="R27" test="/scene/assets/tiles[@id=current()/@tiles]">basemap/@tiles must name a tiles asset.</sch:assert>
    </sch:rule>
    <sch:rule context="tiles">
      <sch:assert id="C46" test="@src or (@url and @cache and @cacheSha256)">tiles need @src, or @url with @cache and @cacheSha256.</sch:assert>
    </sch:rule>
    <sch:rule context="map">
      <sch:assert id="R26" test="not(@fit) or count(str:tokenize(normalize-space(@fit),' ')) = count(/scene/assets/geo[contains(concat(' ',normalize-space(current()/@fit),' '), concat(' ',@id,' '))])">every id in map/@fit must name a geo asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <!-- ============================================================ SREP 8: version gate, references, bounds -->
  <sch:pattern id="p50">
    <sch:rule context="/scene[@version='1.0']">
      <sch:assert id="V6" test="not(.//audioEffect|.//blob|.//burst|.//bus|.//destination|.//erosion|.//flock|.//fluid|.//geo|.//map|.//master|.//param|.//pin|.//poster|.//representation|.//shake|.//slime|.//span|.//thumbnail)">
        version="1.0" documents cannot use 1.1 elements (simulation nodes, geo and map assets, clay blobs,
        audio buses and effects, output posters, thumbnails and destinations, camera shake, representations,
        text spans, effect params); set version="1.1".</sch:assert>
      <sch:assert id="V7" test="not(.//object3D[@primitive='capsule' or @primitive='clay' or @primitive='cone' or @primitive='cylinder' or @primitive='extrude' or @primitive='text' or @primitive='torus'])">
        version="1.0" documents cannot use the 1.1 object3D primitives (capsule, clay, cone, cylinder, extrude, text, torus); set version="1.1".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p51">
    <sch:rule context="*[@colorEnd or @colorHigh or @colorLow or @headFill or @noData or @outline or @paint2]">
      <sch:assert id="R30-colorEnd" test="not(starts-with(@colorEnd,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@colorEnd,'url(#'),')')]">@colorEnd: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-colorHigh" test="not(starts-with(@colorHigh,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@colorHigh,'url(#'),')')]">@colorHigh: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-colorLow" test="not(starts-with(@colorLow,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@colorLow,'url(#'),')')]">@colorLow: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-headFill" test="not(starts-with(@headFill,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@headFill,'url(#'),')')]">@headFill: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-noData" test="not(starts-with(@noData,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@noData,'url(#'),')')]">@noData: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-outline" test="not(starts-with(@outline,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@outline,'url(#'),')')]">@outline: url(#id) must name an element of paints.</sch:assert>
      <sch:assert id="R30-paint2" test="not(starts-with(@paint2,'url(#')) or /scene/paints/*[@id=substring-before(substring-after(current()/@paint2,'url(#'),')')]">@paint2: url(#id) must name an element of paints.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p52">
    <sch:rule context="*[@attenuationColor or @baseColor or @colorEnd or @colorHigh or @colorLow or @emissive or @foreground or @headFill or @keyColor or @noData or @outline or @paint2 or @shadowColor or @sheenColor or @specularColor]">
      <sch:assert id="R31-attenuationColor" test="not(starts-with(@attenuationColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@attenuationColor,'var(--'),')')]">@attenuationColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-baseColor" test="not(starts-with(@baseColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@baseColor,'var(--'),')')]">@baseColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-colorEnd" test="not(starts-with(@colorEnd,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@colorEnd,'var(--'),')')]">@colorEnd: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-colorHigh" test="not(starts-with(@colorHigh,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@colorHigh,'var(--'),')')]">@colorHigh: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-colorLow" test="not(starts-with(@colorLow,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@colorLow,'var(--'),')')]">@colorLow: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-emissive" test="not(starts-with(@emissive,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@emissive,'var(--'),')')]">@emissive: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-foreground" test="not(starts-with(@foreground,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@foreground,'var(--'),')')]">@foreground: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-headFill" test="not(starts-with(@headFill,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@headFill,'var(--'),')')]">@headFill: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-keyColor" test="not(starts-with(@keyColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@keyColor,'var(--'),')')]">@keyColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-noData" test="not(starts-with(@noData,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@noData,'var(--'),')')]">@noData: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-outline" test="not(starts-with(@outline,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@outline,'var(--'),')')]">@outline: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-paint2" test="not(starts-with(@paint2,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@paint2,'var(--'),')')]">@paint2: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-shadowColor" test="not(starts-with(@shadowColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@shadowColor,'var(--'),')')]">@shadowColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-sheenColor" test="not(starts-with(@sheenColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@sheenColor,'var(--'),')')]">@sheenColor: var(--name) must name a styles/token.</sch:assert>
      <sch:assert id="R31-specularColor" test="not(starts-with(@specularColor,'var(--')) or /scene/styles/token[@name=substring-before(substring-after(current()/@specularColor,'var(--'),')')]">@specularColor: var(--name) must name a styles/token.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p53">
    <sch:rule context="*[@shape='sprite'][self::flock or self::particleEmitter]">
      <sch:assert id="C50" test="@sprite">shape="sprite" requires @sprite.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p54">
    <sch:rule context="*[@sprite][self::flock or self::particleEmitter]">
      <sch:assert id="R32" test="/scene/assets/*[self::image or self::imageSequence or self::video or self::generator][@id=current()/@sprite]">@sprite must name an image, image sequence, video or generator asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p55">
    <sch:rule context="erosion[@heightmap]">
      <sch:assert id="R33" test="/scene/assets/image[@id=current()/@heightmap]">erosion/@heightmap must name an image asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p56">
    <sch:rule context="*[@forceFields]">
      <sch:assert id="R34" test="count(str:tokenize(normalize-space(@forceFields),' ')) = count(/scene/physics/forceField[contains(concat(' ',normalize-space(current()/@forceFields),' '), concat(' ',@id,' '))])">
        every id in @forceFields of "<sch:value-of select="@id"/>" must name a physics/forceField.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p57">
    <sch:rule context="*[@textStyle]">
      <sch:assert id="R35" test="/scene/styles/textStyle[@id=current()/@textStyle]">@textStyle must name a styles/textStyle.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p58">
    <sch:rule context="object3D[@primitive='clay']">
      <sch:assert id="C51" test="blob">object3D primitive="clay" needs at least one blob.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p59">
    <sch:rule context="fluidSource[@start and @end]">
      <sch:assert id="C52" test="number(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+')))) &gt; number(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+'))))">fluidSource end must be after start.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p60">
    <sch:rule context="geoLayer[@domain]">
      <sch:assert id="C53" test="not(str:tokenize(normalize-space(@domain),' ')[position() &gt; 1][number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) &lt;= number(substring(normalize-space(preceding-sibling::*[1]),1+number(starts-with(normalize-space(preceding-sibling::*[1]),'+'))))])">@domain values must increase.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p43">
    <sch:rule context="object3D/rigidBody">
      <sch:assert id="C48" test="not(@shape='trimesh') or @type='static' or @type='kinematic'">a trimesh rigidBody must be static or kinematic.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p61">
    <sch:rule context="output">
      <sch:let name="mixTracks" value="/scene/audioMix/audioTrack/@id"/>
      <sch:let name="mixBuses" value="/scene/audioMix/bus/@id"/>
      <sch:assert id="C54" test="not(segment) or ((not(@start) or number(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+')))) = 0) and not(@end))">an output with segments cannot also set start or end; put the range in a segment instead.</sch:assert>
      <sch:assert id="R39" test="not(str:tokenize(normalize-space(@audioTracks),' ')[not(. = $mixTracks)]) and not(str:tokenize(normalize-space(@audioBuses),' ')[not(. = $mixBuses)])">every id in output/@audioTracks must name an audioMix track, and every id in output/@audioBuses a bus.</sch:assert>
      <sch:assert id="R40" test="not(@overlay) or /scene/symbols/symbol[@id=current()/@overlay]">output/@overlay must name a symbol.</sch:assert>
    </sch:rule>
    <sch:rule context="segment">
      <sch:assert id="C55" test="timeRemap or ((@from or @fromMarker) and (@to or @toMarker))">a segment needs from (or fromMarker) and to (or toMarker), or a timeRemap.</sch:assert>
      <sch:assert id="C56" test="not(@from and @to) or (number(substring(normalize-space(@from),1+number(starts-with(normalize-space(@from),'+')))) &gt;= 0 and number(substring(normalize-space(@to),1+number(starts-with(normalize-space(@to),'+')))) &gt; number(substring(normalize-space(@from),1+number(starts-with(normalize-space(@from),'+')))) and number(substring(normalize-space(@to),1+number(starts-with(normalize-space(@to),'+')))) &lt;= number(substring(normalize-space(/scene/project/@duration),1+number(starts-with(normalize-space(/scene/project/@duration),'+')))))">segment from and to must satisfy 0 ≤ from &lt; to ≤ project/@duration.</sch:assert>
      <sch:assert id="C57" test="not(@from and @fromMarker) and not(@to and @toMarker)">a segment gives each end as a time or as a marker, not both.</sch:assert>
      <sch:assert id="C58" test="count(timeRemap) &lt;= 1 and count(transition) &lt;= 1">a segment has at most one timeRemap and one transition.</sch:assert>
      <sch:assert id="R38" test="(not(@fromMarker) or /scene/markers/marker[@id=current()/@fromMarker]) and (not(@toMarker) or /scene/markers/marker[@id=current()/@toMarker])">segment markers must name markers.</sch:assert>
    </sch:rule>
    <sch:rule context="segment/transition">
      <sch:assert id="C59" test="not(@from or @to) and not(@type='morph' or @type='luma')">a segment transition joins two rendered pictures: no from, no to, not morph and not luma.</sch:assert>
    </sch:rule>
    <sch:rule context="output/captionTrack[@transcribe]">
      <sch:assert id="R41" test="../audioTrack[@id=current()/@transcribe]">an output caption track transcribes one of that output's own audio tracks.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p66">
    <sch:rule context="shape[@markerStart[.!='none'] or @markerEnd[.!='none']]">
      <sch:assert id="C65" test="@shape='path' or @shape='line'">markers need an open outline: shape="path" or "line".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p70">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V11" test="not(.//connector)">connector needs version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p71">
    <sch:rule context="connector">
      <sch:assert id="C60" test="(@from or (@fromX and @fromY)) and (@to or (@toX and @toY))">a connector end needs a node (@from, @to) or a point (@fromX and @fromY, @toX and @toY).</sch:assert>
      <sch:assert id="C61" test="not(@fromAnchor[.!='auto'] and (@fromX or @fromY)) and not(@toAnchor[.!='auto'] and (@toX or @toY))">an anchor keyword and an explicit anchor point exclude each other.</sch:assert>
      <sch:assert id="C62" test="count(@fromX|@fromY) != 1 and count(@toX|@toY) != 1">@fromX and @fromY (and @toX and @toY) come together.</sch:assert>
      <sch:assert id="C63" test="not(@route='curved' and @points)">route="curved" takes no @points.</sch:assert>
      <sch:assert id="C64" test="not(animate[@property='x' or @property='y' or @property='rotation' or @property='scaleX' or @property='scaleY' or @property='anchorX' or @property='anchorY' or @property='skewX' or @property='skewY']) and not(expression[@property!='opacity'])">a connector has no transform of its own: its geometry comes from its ends.</sch:assert>
      <sch:assert id="R48-from" test="not(@from) or (ancestor::symbol and ancestor::symbol[1]//*[@id=current()/@from][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[normalize-space(@threeD)='true' or normalize-space(@threeD)='1'])]) or (not(ancestor::symbol) and /scene/composition//*[@id=current()/@from][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[normalize-space(@threeD)='true' or normalize-space(@threeD)='1'])])">@from must name a group, layer, shape or instance in the same composition or symbol, outside any repeat and not 2.5D.</sch:assert>
      <sch:assert id="R48-to" test="not(@to) or (ancestor::symbol and ancestor::symbol[1]//*[@id=current()/@to][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[normalize-space(@threeD)='true' or normalize-space(@threeD)='1'])]) or (not(ancestor::symbol) and /scene/composition//*[@id=current()/@to][self::group or self::layer or self::shape or self::instance][not(ancestor::repeat)][not(ancestor-or-self::*[normalize-space(@threeD)='true' or normalize-space(@threeD)='1'])])">@to must name a group, layer, shape or instance in the same composition or symbol, outside any repeat and not 2.5D.</sch:assert>
      <sch:assert id="R49" test="not(//transformConstraint[@target=current()/@id] | //*[@parent=current()/@id] | //link[starts-with(@source, concat(current()/@id, '.'))])">nothing may be positioned by a connector (transform parent, constraint target, link source).</sch:assert>
      <sch:assert id="R50" test="not(@label) or /scene/assets/text[@id=current()/@label]">@label must name a text asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p72">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V9" test="not(assets/pdf)">pdf assets need version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p73">
    <sch:rule context="assets/pdf">
      <sch:assert id="C66" test="@sha256">a pdf asset pins its source with @sha256.</sch:assert>
    </sch:rule>
    <sch:rule context="shape[@region]">
      <sch:assert id="C67" test="@regionLayer">@region needs @regionLayer, the layer that shows the page.</sch:assert>
      <sch:assert id="C68" test="not(@parent) and not(transformConstraint)">a shape placed on a region has no other transform parent or constraint.</sch:assert>
      <sch:assert id="R51" test="/scene/assets/pdf/region[@id=current()/@region]">@region must name a region of a pdf asset.</sch:assert>
      <sch:assert id="R52" test="//layer[@id=current()/@regionLayer][@asset=/scene/assets/pdf[region/@id=current()/@region]/@id]">@regionLayer must name a layer whose asset holds the region.</sch:assert>
    </sch:rule>
    <sch:rule context="shape">
      <sch:assert id="C69" test="@shape='parametric' or (@width and @height)">shape needs @width and @height unless it takes its box from @region or is parametric.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p74">
    <sch:rule context="/scene[project/@fontPolicy='pinned']">
      <sch:assert id="C70" test="not(//@fontFile)">fontPolicy="pinned": faces come from font assets, not @fontFile.</sch:assert>
      <sch:assert id="C71" test="not(assets/font[not(@sha256)])">fontPolicy="pinned": every font asset carries @sha256.</sch:assert>
      <sch:assert id="C72" test="not(//@font[not(. = /scene/assets/font/@family)])">fontPolicy="pinned": every @font names the family of a font asset.</sch:assert>
      <sch:assert id="C73" test="not(//@fallback[str:tokenize(., ',')[not(normalize-space(.) = current()/assets/font/@family)]])">fontPolicy="pinned": every @fallback family names a font asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p75">
    <sch:rule context="/scene[@version='1.0' or @version='1.1']">
      <sch:assert id="V10" test="not(.//repeat/points)">points in a repeat needs version="1.2".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p76">
    <sch:rule context="repeat">
      <sch:assert id="C74" test="count(points) &lt;= 1">a repeat takes at most one points child.</sch:assert>
      <sch:assert id="C80" test="not(points and (@from[number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) != 0] or @step[number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) != 1]))">a repeat with a points child takes no @from other than 0 and no @step other than 1.</sch:assert>
    </sch:rule>
    <sch:rule context="repeat/points">
      <sch:assert id="C75" test="not(@type='along-path') or (@path and @count)">points type="along-path" needs @path and @count.</sch:assert>
      <sch:assert id="C76" test="not(@type='scatter') or (@count and ((@path and not(@width or @height)) or (not(@path) and @width and @height)))">points type="scatter" needs @count and either @path or both @width and @height.</sch:assert>
      <sch:assert id="C77" test="not(@type='vertices') or @path">points type="vertices" needs @path.</sch:assert>
      <sch:assert id="C78" test="not(@type='list') or @at">points type="list" needs @at.</sch:assert>
      <sch:assert id="C79" test="not(animate[not(@property='spacingX' or @property='spacingY' or @property='width' or @property='height')])">points animates only spacingX, spacingY, width and height.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p62">
    <sch:rule context="paints/pattern">
      <sch:assert id="R42" test="/scene/assets/image[@id=current()/@asset]">pattern/@asset must name an image asset: a pattern tiles an image.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p63">
    <sch:rule context="*[@emitterAsset]">
      <sch:assert id="R43" test="/scene/assets/image[@id=current()/@emitterAsset]">@emitterAsset must name an image asset: particles are emitted from its opaque pixels.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p64">
    <sch:rule context="effect[@source][@type='displacement-map' or @type='difference-key' or @type='shader']">
      <sch:assert id="R44" test="/scene/composition//*[@id=current()/@source] or /scene/symbols//*[@id=current()/@source]">effect @source must name a composition node; an asset is placed on a (hidden) layer, and the layer named.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p65">
    <sch:rule context="generator[@lineWidth]">
      <sch:assert id="R47" test="@kind='grid'">@lineWidth is the line width of a grid generator.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p67">
    <sch:rule context="object3D[@tracking]">
      <sch:assert id="TXT2" test="@primitive='text'">@tracking applies to object3D primitive="text".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p68">
    <sch:rule context="object3D[@materialOverride]">
      <sch:assert id="MOV1" test="count(str:tokenize(normalize-space(@materialOverride),' ')) &gt; 0 and count(str:tokenize(normalize-space(@materialOverride),' ')) = count(str:tokenize(normalize-space(@materialOverride),' ')[contains(.,':') and substring-before(.,':')!='' and substring-after(.,':')!=''])">@materialOverride is a space-separated list of name:id pairs.</sch:assert>
      <sch:assert id="MOV2" test="not(str:tokenize(normalize-space(@materialOverride),' ')[not(substring-after(., ':') = current()/ancestor::scene/materials/material/@id)])">@materialOverride: each id after the colon must name a material.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p69">
    <sch:rule context="shape[@shape='stroke-text']">
      <sch:assert id="PEN1" test="@text and @strokeFont">shape="stroke-text" needs @text and @strokeFont.</sch:assert>
      <sch:assert id="PEN2" test="not(@strokeFont) or /scene/assets/strokeFont[@id=current()/@strokeFont]">shape/@strokeFont must name a strokeFont asset.</sch:assert>
    </sch:rule>
  </sch:pattern>

  <!-- ===================================================== volumetric and large-scale cinematic effects (SREP 40, version 1.3) -->
  <sch:pattern id="cinematic-volume">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2']">
      <sch:assert id="V8" test="not(assets/volume|.//medium|.//pyro|.//object3D[@primitive='volume' or @volume])">volumetric assets and media require version="1.3" or later.</sch:assert>
    </sch:rule>
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5' or @version='1.6' or @version='1.7']">
      <sch:assert id="MED4" test="not(.//medium[@phase or @phaseAlpha or @dropletDiameter])">medium phase, phaseAlpha and dropletDiameter require version="1.8" or later.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D">
      <sch:assert id="VOL1" test="not(@primitive='volume') or count(@volume|pyro)=1">volume primitive requires exactly one source: @volume or a pyro child.</sch:assert>
      <sch:assert id="VOL2" test="not(@volume) or /scene/assets/volume[@id=current()/@volume]">object3D/@volume must name a volume asset.</sch:assert>
      <sch:assert id="VOL3" test="count(medium)&lt;=1 and (@primitive='volume' or (not(medium) and not(@volume) and not(pyro)))">one medium child and @volume are permitted only on a volume primitive.</sch:assert>
      <sch:assert id="VOL5" test="not(medium[normalize-space(@blackbody)='true' or normalize-space(@blackbody)='1']) or pyro or /scene/assets/volume[@id=current()/@volume]/@temperatureGrid">blackbody emission requires a declared temperatureGrid on the volume asset.</sch:assert>
      <sch:assert id="VOL10" test="not(medium[@lightGridCell or @lightGridDomeDirections or @lightGridMemoryMiB]) or medium/@lighting='grid'">the lightGrid* attributes of a medium apply only with lighting="grid".</sch:assert>
      <sch:assert id="MED1" test="not(medium/@phaseAlpha) or medium/@phase='draine'">medium phaseAlpha applies only with phase="draine".</sch:assert>
      <sch:assert id="MED2" test="not(medium/@dropletDiameter) or medium/@phase='fog'">medium dropletDiameter applies only with phase="fog".</sch:assert>
      <sch:assert id="MED3" test="not(medium[@phase='rayleigh' or @phase='fog']/@anisotropy) or number(medium/@anisotropy)=0">medium anisotropy is absent or 0 with phase="rayleigh" and phase="fog", whose shapes are fixed.</sch:assert>
    </sch:rule>
    <sch:rule context="pyro">
      <sch:let name="nx" value="number(substring(normalize-space(@width),1+number(starts-with(normalize-space(@width),'+')))) div number(substring(normalize-space(@voxelSize),1+number(starts-with(normalize-space(@voxelSize),'+'))))"/>
      <sch:let name="ny" value="number(substring(normalize-space(@height),1+number(starts-with(normalize-space(@height),'+')))) div number(substring(normalize-space(@voxelSize),1+number(starts-with(normalize-space(@voxelSize),'+'))))"/>
      <sch:let name="nz" value="number(substring(normalize-space(@depth),1+number(starts-with(normalize-space(@depth),'+')))) div number(substring(normalize-space(@voxelSize),1+number(starts-with(normalize-space(@voxelSize),'+'))))"/>
      <sch:let name="colliders" value="/scene//object3D[contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))]"/>
      <sch:let name="count" value="count(str:tokenize(normalize-space(@colliders),' '))"/>
      <sch:assert id="PYRO1" test="$nx&gt;=2 and $nx&lt;=1024 and $ny&gt;=2 and $ny&lt;=1024 and $nz&gt;=2 and $nz&lt;=1024 and $nx - round($nx)&lt;=0.00000001 and round($nx) - $nx&lt;=0.00000001 and $ny - round($ny)&lt;=0.00000001 and round($ny) - $ny&lt;=0.00000001 and $nz - round($nz)&lt;=0.00000001 and round($nz) - $nz&lt;=0.00000001">pyro dimensions must be integer multiples of voxelSize, with 2..1024 cells per axis.</sch:assert>
      <sch:assert id="PYRO7" test="not(@colliders) or ($count &lt;= 4096 and $count = count($colliders[@primitive='box' or @primitive='sphere' or @primitive='globe' or @primitive='plane' or @primitive='cylinder' or @primitive='cone' or @primitive='capsule' or @primitive='torus' or @primitive='mesh' or @primitive='text' or @primitive='extrude' or @primitive='clay']))">pyro colliders must name at most 4096 distinct supported surface objects, including text, extrude and clay solids.</sch:assert>
      <sch:assert id="PYRO8" test="not($colliders/*[@property='primitive' or @property='mesh' or @property='radius' or @property='width' or @property='height' or @property='depth' or @property='segments' or (@property='exaggeration' and ../@primitive='globe' and ../@terrain) or (../@primitive='text' and (@property='text' or @property='font' or @property='bevel' or @property='tracking')) or (../@primitive='extrude' and (@property='path' or @property='bevel')) or (../@primitive='clay' and (@property='resolution' or @property='fingerprints' or @property='seed' or @property='boil'))]) and not($colliders[@primitive='clay']/blob/*[@property]) and not($colliders[@primitive='clay' and number(translate(@boil,'+',''))&gt;0 and number(translate(@fingerprints,'+',''))&gt;0])">pyro collider geometry is static; animate its position, rotation or scale instead of shape parameters.</sch:assert>
      <sch:assert id="PYRO9" test="not(normalize-space(@follow)='true' or normalize-space(@follow)='1') or @boundary='open'">a pyro whose window follows its plume needs boundary="open".</sch:assert>
      <sch:assert id="PYRO10" test="normalize-space(@follow)='true' or normalize-space(@follow)='1' or not(@followMargin or @followLoss)">followMargin and followLoss belong to a pyro that follows its plume.</sch:assert>
      <sch:assert id="PYRO11" test="not(@followMargin) or (2 * number(substring(normalize-space(@followMargin),1+number(starts-with(normalize-space(@followMargin),'+')))) * number(substring(normalize-space(@voxelSize),1+number(starts-with(normalize-space(@voxelSize),'+')))) &lt; number(substring(normalize-space(@width),1+number(starts-with(normalize-space(@width),'+')))) and 2 * number(substring(normalize-space(@followMargin),1+number(starts-with(normalize-space(@followMargin),'+')))) * number(substring(normalize-space(@voxelSize),1+number(starts-with(normalize-space(@voxelSize),'+')))) &lt; number(substring(normalize-space(@height),1+number(starts-with(normalize-space(@height),'+')))) and 2 * number(substring(normalize-space(@followMargin),1+number(starts-with(normalize-space(@followMargin),'+')))) * number(substring(normalize-space(@voxelSize),1+number(starts-with(normalize-space(@voxelSize),'+')))) &lt; number(substring(normalize-space(@depth),1+number(starts-with(normalize-space(@depth),'+')))))">followMargin must leave a cell between the faces of the window on every axis.</sch:assert>
      <sch:assert id="PYC5" test="not(pyroBlast) or @boundary='open'">a pyro with a blast needs boundary="open": a blast is a source of divergence, which a closed domain cannot let out.</sch:assert>
    </sch:rule>
    <sch:rule context="pyroBlast">
      <sch:assert id="PYC6" test="(not(@x) or (number(substring(normalize-space(@x),1+number(starts-with(normalize-space(@x),'+')))) &gt;= -number(substring(normalize-space(../@width),1+number(starts-with(normalize-space(../@width),'+')))) div 2 and number(substring(normalize-space(@x),1+number(starts-with(normalize-space(@x),'+')))) &lt;= number(substring(normalize-space(../@width),1+number(starts-with(normalize-space(../@width),'+')))) div 2)) and (not(@y) or (number(substring(normalize-space(@y),1+number(starts-with(normalize-space(@y),'+')))) &gt;= -number(substring(normalize-space(../@height),1+number(starts-with(normalize-space(../@height),'+')))) div 2 and number(substring(normalize-space(@y),1+number(starts-with(normalize-space(@y),'+')))) &lt;= number(substring(normalize-space(../@height),1+number(starts-with(normalize-space(../@height),'+')))) div 2)) and (not(@z) or (number(substring(normalize-space(@z),1+number(starts-with(normalize-space(@z),'+')))) &gt;= -number(substring(normalize-space(../@depth),1+number(starts-with(normalize-space(../@depth),'+')))) div 2 and number(substring(normalize-space(@z),1+number(starts-with(normalize-space(@z),'+')))) &lt;= number(substring(normalize-space(../@depth),1+number(starts-with(normalize-space(../@depth),'+')))) div 2))">a blast is released inside the domain of its pyro (x between plus and minus half the width, and likewise y with the height and z with the depth).</sch:assert>
    </sch:rule>
    <sch:rule context="pyroSource|pyroImpulse">
      <sch:assert id="PYRO5" test="(@shape='mesh' and /scene/assets/mesh[@id=current()/@mesh]) or (not(@shape='mesh') and not(@mesh))">a mesh pyro source must name a mesh asset; @mesh is valid only for shape=mesh.</sch:assert>
      <sch:assert id="PYRO6" test="not(*[@property='shape' or @property='mesh'])">pyro shape and mesh selection are static; animate the source transform instead.</sch:assert>
      <sch:assert id="PYRO2" test="not(@end) or (@start and number(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+'))))&gt;number(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+'))))) or (not(@start) and number(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+'))))&gt;0)">pyro source end must be greater than start.</sch:assert>
      <sch:assert id="PYRO3" test="not(@shape='box') or (@width and @height and @depth)">box pyro sources require width, height and depth.</sch:assert>
      <sch:assert id="PYRO4" test="not(number(substring(normalize-space(@scaleX),1+number(starts-with(normalize-space(@scaleX),'+'))))=0 or number(substring(normalize-space(@scaleY),1+number(starts-with(normalize-space(@scaleY),'+'))))=0 or number(substring(normalize-space(@scaleZ),1+number(starts-with(normalize-space(@scaleZ),'+'))))=0)">pyro source transforms must be invertible (nonzero scales).</sch:assert>
      <sch:assert id="PYC1" test="not(@crater and (@start or @end or @time or @densityRate or @temperatureRate or @density or @temperature or @expansion or @shape or @mesh or @radius or @width or @height or @depth or @x or @y or @z or @rotation or @rotationX or @rotationY or @scaleX or @scaleY or @scaleZ))">a pyro source or impulse from a crater derives its shape, place, timing, density, temperature and expansion, so none of them may be given.</sch:assert>
      <sch:assert id="PYC2" test="@crater or not(@heatFraction or @dustFraction or @specificHeat or @maxTemperature)">heatFraction, dustFraction, specificHeat and maxTemperature belong to a pyro source or impulse from a crater.</sch:assert>
      <sch:assert id="PYC3" test="not(self::pyroImpulse) or @time or @crater">a pyro impulse requires time unless it comes from a crater.</sch:assert>
      <sch:assert id="PYC4" test="not(@crater) or /scene//crater[@id=current()/@crater and @source]">a pyro source or impulse from a crater must name a crater that grows from an impact.</sch:assert>
    </sch:rule>
    <sch:rule context="assets/volume">
      <sch:let name="first" value="number(substring(normalize-space(@first),1+number(starts-with(normalize-space(@first),'+'))))"/>
      <sch:let name="last" value="number(substring(normalize-space(@last),1+number(starts-with(normalize-space(@last),'+'))))"/>
      <sch:let name="fmt" value="substring-before(substring-after(@src,'%'),'d')"/>
      <sch:let name="tail" value="substring-after(@src,'#')"/>
      <sch:let name="nonhash" value="translate($tail,'#','')"/>
      <sch:let name="pattern" value="(contains(@src,'%') and contains(substring-after(@src,'%'),'d') and translate($fmt,'0123456789','')='' and ($fmt='' or number($fmt)&lt;=64)) or (not(contains(@src,'%')) and contains(@src,'#') and (($nonhash='' and string-length($tail)&lt;64) or ($nonhash!='' and string-length(substring-before($tail,substring($nonhash,1,1)))&lt;64)))"/>
      <sch:assert id="VOL6" test="@format='srvseq' or ((@first or @last) and @first and @last and $last&gt;=$first and $last - $first&lt;1000000 and $pattern) or (not(@first or @last) and not(@fps) and (not(@interpolation) or @interpolation='hold') and (not(@missingFrame) or @missingFrame='error'))">volume sequences require first/last, 1..1000000 ordered frames and a numbered src pattern; sequence options require a sequence.</sch:assert>
      <sch:assert id="VOL9" test="(@interpolation='advect' and count(@velocityGridX|@velocityGridY|@velocityGridZ)=3) or ((not(@interpolation) or @interpolation!='advect') and not(@velocityGridX|@velocityGridY|@velocityGridZ))">advect requires all three velocity channels; velocity channels require advect interpolation.</sch:assert>
      <sch:assert id="VOL8" test="not(@format='srvseq') or (@sha256 and not(@first or @last or @fps) and (not(@missingFrame) or @missingFrame='error'))">srvseq requires sha256 and owns its frame range and composition clock; first, last, fps and missing-frame substitution are not permitted.</sch:assert>
      <sch:assert id="VOL7" test="not(@first or @last) or not(@sha256)">volume sha256 applies to a single cache file, not a numbered sequence.</sch:assert>
      <sch:assert id="VOL4" test="not(@boundsMinX|@boundsMinY|@boundsMinZ|@boundsMaxX|@boundsMaxY|@boundsMaxZ) or (count(@boundsMinX|@boundsMinY|@boundsMinZ|@boundsMaxX|@boundsMaxY|@boundsMaxZ)=6 and number(substring(normalize-space(@boundsMinX),1+number(starts-with(normalize-space(@boundsMinX),'+'))))&lt;number(substring(normalize-space(@boundsMaxX),1+number(starts-with(normalize-space(@boundsMaxX),'+')))) and number(substring(normalize-space(@boundsMinY),1+number(starts-with(normalize-space(@boundsMinY),'+'))))&lt;number(substring(normalize-space(@boundsMaxY),1+number(starts-with(normalize-space(@boundsMaxY),'+')))) and number(substring(normalize-space(@boundsMinZ),1+number(starts-with(normalize-space(@boundsMinZ),'+'))))&lt;number(substring(normalize-space(@boundsMaxZ),1+number(starts-with(normalize-space(@boundsMaxZ),'+')))))">volume bounds require all six finite coordinates with each minimum below its maximum.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-voxels-version">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2']">
      <sch:assert id="VOX1" test="not(assets/voxelAsset|.//object3D[@primitive='voxels' or @voxels])">voxel assets and the voxels primitive require version="1.3" or later.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-voxel-assets">
    <sch:rule context="assets/voxelAsset">
      <sch:assert id="VOX2" test="count(@src|@fromMesh)=1 and (not(@format|@model) or @src) and (not(@model) or @format='vox' or (not(@format) and substring(@src,string-length(@src) - 3)='.vox')) and (not(@voxelGrid) or @format='srvol' or (not(@format) and substring(@src,string-length(@src) - 5)='.srvol')) and ((@fromMesh and @cellSize) or not(@fromMesh|@cellSize))">a voxelAsset has exactly one of src and fromMesh; format, model and voxelGrid belong to src (model to a vox file, voxelGrid to an srvol file), and cellSize is required with fromMesh and not given with src.</sch:assert>
      <sch:assert id="VOX3" test="not(@fromMesh) or /scene/assets/mesh[@id=current()/@fromMesh]">voxelAsset/@fromMesh must name a mesh asset.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-voxel-objects">
    <sch:rule context="object3D">
      <sch:let name="materials" value="/scene/materials/material"/>
      <sch:assert id="VOX4" test="not(@primitive='voxels') or (@voxels and /scene/assets/voxelAsset[@id=current()/@voxels])">an object3D of primitive voxels names a voxelAsset in voxels.</sch:assert>
      <sch:assert id="VOX5" test="@primitive='voxels' or not(@voxels|@cellSize|@palette|@surface|@surfaceMemoryMiB)">voxels, cellSize, palette, surface and surfaceMemoryMiB belong to primitive="voxels".</sch:assert>
      <sch:assert id="VOX6" test="not(@palette) or normalize-space(@palette)='file' or (count(str:tokenize(normalize-space(@palette),' '))&lt;=255 and count(str:tokenize(normalize-space(@palette),' ')[. = $materials/@id])=count(str:tokenize(normalize-space(@palette),' ')))">palette is the word file or at most 255 material ids.</sch:assert>
      <sch:assert id="VOX7" test="not(@primitive='voxels') or not(@mesh|@volume|@terrain|@map|@text|@path|medium|pyro)">a voxels object has no mesh, volume, terrain, map, text or path, and no medium or pyro child.</sch:assert>
      <sch:assert id="VOX15" test="not(@primitive='voxels' and (crater or fracture)) or (rigidBody and not(rigidBody[@shape and not(@shape='auto' or @shape='voxels')]))">an object of cells that a crater or a fracture breaks has a rigidBody whose collider is the cells (shape voxels or auto, or no shape).</sch:assert>
    </sch:rule>
    <sch:rule context="object3D/rigidBody">
      <sch:let name="voxels" value="boolean(parent::object3D[@primitive='voxels'])"/>
      <sch:let name="cells" value="boolean(parent::object3D[@primitive='voxels'] and (not(@shape) or @shape='auto' or @shape='voxels'))"/>
      <sch:let name="sx" value="number(concat(substring(normalize-space(../@scaleX), 1 + number(starts-with(normalize-space(../@scaleX), '+'))), substring('1', 1 + string-length(normalize-space(../@scaleX)))))"/>
      <sch:let name="sy" value="number(concat(substring(normalize-space(../@scaleY), 1 + number(starts-with(normalize-space(../@scaleY), '+'))), substring('1', 1 + string-length(normalize-space(../@scaleY)))))"/>
      <sch:let name="sz" value="number(concat(substring(normalize-space(../@scaleZ), 1 + number(starts-with(normalize-space(../@scaleZ), '+'))), substring('1', 1 + string-length(normalize-space(../@scaleZ)))))"/>
      <sch:assert id="VOX8" test="not(@shape='voxels') or $voxels">a rigidBody with shape voxels belongs to an object3D of primitive voxels.</sch:assert>
      <sch:assert id="VOX9" test="$cells or not(@density or @maxFragments or @fragmentMinCells or @fragmentOverflow or @anchor)">density, maxFragments, fragmentMinCells, fragmentOverflow and anchor belong to a rigidBody whose collider is the cells of an object of primitive voxels.</sch:assert>
      <sch:assert id="VOX10" test="not($cells) or (@density and not(@mass))">a body of cells has a density and no mass (its mass is its cells').</sch:assert>
      <sch:assert id="VOX11" test="not(@maxFragments or @fragmentMinCells or @fragmentOverflow) or ../crater or ../fracture">maxFragments, fragmentMinCells and fragmentOverflow belong to a body of cells that a crater or a fracture can break.</sch:assert>
      <sch:assert id="VOX12" test="not(@anchor) or ../crater">anchor belongs to a body of cells that has a crater.</sch:assert>
      <sch:assert id="VOX13" test="not($cells and (../crater or ../fracture)) or ($sx = $sy and $sy = $sz)">a body of cells that a crater or a fracture breaks is scaled the same on every axis.</sch:assert>
      <sch:assert id="VOX14" test="not($voxels and ../crater and ../fracture)">a body of cells has a crater or a fracture, not both.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-mesh-sequence">
    <sch:rule context="assets/meshSequence">
      <sch:let name="first" value="number(substring(normalize-space(@first),1+number(starts-with(normalize-space(@first),'+'))))"/>
      <sch:let name="last" value="number(substring(normalize-space(@last),1+number(starts-with(normalize-space(@last),'+'))))"/>
      <sch:let name="fmt" value="substring-before(substring-after(@src,'%'),'d')"/>
      <sch:let name="tail" value="substring-after(@src,'#')"/>
      <sch:let name="nonhash" value="translate($tail,'#','')"/>
      <sch:let name="pattern" value="(contains(@src,'%') and contains(substring-after(@src,'%'),'d') and translate($fmt,'0123456789','')='' and ($fmt='' or number($fmt)&lt;=64)) or (not(contains(@src,'%')) and contains(@src,'#') and (($nonhash='' and string-length($tail)&lt;64) or ($nonhash!='' and string-length(substring-before($tail,substring($nonhash,1,1)))&lt;64)))"/>
      <sch:assert id="MSQ1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2')">mesh sequences require version="1.3" or later.</sch:assert>
      <sch:assert id="MSQ2" test="$last&gt;=$first and $last - $first&lt;1000000 and $pattern">mesh sequences require 1..1000000 ordered frames and a numbered src pattern.</sch:assert>
      <sch:assert id="MSQ3" test="not(@sha256)">mesh sequence sha256 cannot identify multiple numbered files.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D|pyro|particles3D">
      <sch:assert id="MSQ4" test="not(self::object3D[@primitive='mesh' and rigidBody and not(fracture and rigidBody[@type='static' or @type='kinematic']) and @mesh=/scene/assets/meshSequence/@id]) and not(/scene//object3D[@primitive='mesh' and @mesh=/scene/assets/meshSequence/@id and contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))])">mesh sequences are changing render geometry; rigid collider consumers require an explicit static proxy.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-particles">
    <sch:rule context="particles3D">
      <sch:let name="start" value="number(concat(substring(normalize-space(@emissionStart),1+number(starts-with(normalize-space(@emissionStart),'+'))),substring('0',1,number(not(@emissionStart)))))"/>
      <sch:let name="life" value="number(concat(substring(normalize-space(@lifetime),1+number(starts-with(normalize-space(@lifetime),'+'))),substring('0',1,number(not(@lifetime))))) + 2 * not(@lifetime)"/>
      <sch:let name="dy" value="number(concat(substring(normalize-space(@directionY),1+number(starts-with(normalize-space(@directionY),'+'))),substring('0',1,number(not(@directionY))))) - not(@directionY)"/>
      <sch:let name="colliders" value="/scene//object3D[contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))]"/>
      <sch:assert id="P3D1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2')">particles3D requires version="1.3" or later.</sch:assert>
      <sch:assert id="P3D2" test="((@emitterShape='mesh' and /scene/assets/mesh[@id=current()/@emitterMesh]) or (not(@emitterShape='mesh') and not(@emitterMesh))) and ((@shape='mesh' and /scene/assets/mesh[@id=current()/@mesh]) or (not(@shape='mesh') and not(@mesh))) and (not(@sprite) or (@shape='billboard' and /scene/assets/image[@id=current()/@sprite])) and (not(@material) or /scene/materials/material[@id=current()/@material])">particles3D mesh, emitterMesh, sprite and material must name the appropriate assets and match the selected shapes.</sch:assert>
      <sch:assert id="P3D3" test="number(concat(substring(normalize-space(@lifetimeVariance),1+number(starts-with(normalize-space(@lifetimeVariance),'+'))),substring('0',1,number(not(@lifetimeVariance)))))&lt;$life and number(concat(substring(normalize-space(@speedVariance),1+number(starts-with(normalize-space(@speedVariance),'+'))),substring('0',1,number(not(@speedVariance)))))&lt;=number(concat(substring(normalize-space(@speed),1+number(starts-with(normalize-space(@speed),'+'))),substring('0',1,number(not(@speed))))) and (number(concat(substring(normalize-space(@directionX),1+number(starts-with(normalize-space(@directionX),'+'))),substring('0',1,number(not(@directionX)))))!=0 or $dy!=0 or number(concat(substring(normalize-space(@directionZ),1+number(starts-with(normalize-space(@directionZ),'+'))),substring('0',1,number(not(@directionZ)))))!=0) and (not(@emissionEnd) or number(substring(normalize-space(@emissionEnd),1+number(starts-with(normalize-space(@emissionEnd),'+'))))&gt;=$start) and not(burst[not(@crater) and not(number(substring(normalize-space(@time),1+number(starts-with(normalize-space(@time),'+'))))&gt;=$start)])">particles3D requires bounded variances, a nonzero direction, an ordered emission window and bursts at or after emissionStart.</sch:assert>
      <sch:assert id="P3D4" test="not(*[@property and not(@property='x' or @property='y' or @property='z' or @property='rotation' or @property='rotationX' or @property='rotationY' or @property='scaleX' or @property='scaleY' or @property='scaleZ' or @property='rate' or @property='opacity' or @property='size' or @property='sizeEnd' or @property='color' or @property='colorEnd' or @property='opacityEnd' or @property='trail')])">particles3D solver configuration is static; animate pose, rate or appearance instead.</sch:assert>
      <sch:assert id="P3D5" test="not(@colliders) or (count(str:tokenize(normalize-space(@colliders),' '))&lt;=4096 and count(str:tokenize(normalize-space(@colliders),' '))=count($colliders[@primitive='box' or @primitive='sphere' or @primitive='globe' or @primitive='plane' or @primitive='cylinder' or @primitive='cone' or @primitive='capsule' or @primitive='torus' or @primitive='mesh' or @primitive='text' or @primitive='extrude' or @primitive='clay']))">particles3D colliders must name at most 4096 distinct supported rigid surface objects.</sch:assert>
      <sch:assert id="P3D11" test="not(@gas) or /scene//object3D[@id=current()/@gas]/pyro">particles3D gas must name an object3D that holds a native pyro volume; a baked or asset volume has no velocity grid.</sch:assert>
      <sch:assert id="P3D6" test="not($colliders/*[@property='primitive' or @property='mesh' or @property='radius' or @property='width' or @property='height' or @property='depth' or @property='segments' or (@property='exaggeration' and ../@primitive='globe' and ../@terrain) or (../@primitive='text' and (@property='text' or @property='font' or @property='bevel' or @property='tracking')) or (../@primitive='extrude' and (@property='path' or @property='bevel')) or (../@primitive='clay' and (@property='resolution' or @property='fingerprints' or @property='seed' or @property='boil'))]) and not($colliders[@primitive='clay']/blob/*[@property]) and not($colliders[@primitive='clay' and number(translate(@boil,'+',''))&gt;0 and number(translate(@fingerprints,'+',''))&gt;0])">particles3D collider geometry must be static; animate its rigid pose instead.</sch:assert>
    </sch:rule>
    <sch:rule context="particles3D/burst">
      <sch:let name="angle" value="number(concat(substring(normalize-space(@angle),1+number(starts-with(normalize-space(@angle),'+'))),substring('0',1,number(not(@angle))))) + 45 * not(@angle)"/>
      <sch:let name="spread" value="number(concat(substring(normalize-space(@angleSpread),1+number(starts-with(normalize-space(@angleSpread),'+'))),substring('0',1,number(not(@angleSpread))))) + 15 * not(@angleSpread)"/>
      <sch:assert id="P3D7" test="(@crater and not(@time or @repeat or @interval)) or (not(@crater) and @time)">a burst requires time unless it comes from a crater, and a burst from a crater derives its instants, so time, repeat and interval may not be given.</sch:assert>
      <sch:assert id="P3D8" test="not(@crater) or /scene//crater[@id=current()/@crater and @source]">a burst from a crater must name a crater that grows from an impact.</sch:assert>
      <sch:assert id="P3D9" test="@crater or not(@angle or @angleSpread)">angle and angleSpread belong to a burst from a crater.</sch:assert>
      <sch:assert id="P3D10" test="not(@crater) or ($angle - $spread &gt;= 0 and $angle + $spread &lt;= 90)">the launch angle of a burst from a crater (default 45) and its spread (default 15) must stay between 0 and 90 degrees.</sch:assert>
      <sch:assert id="CRT17" test="not(@crater and /scene//object3D[@primitive='voxels' and crater/@id=current()/@crater]) or not(@angle or @angleSpread)">a burst from the crater of an object of cells launches the cells that the cut throws with the cut's own velocities, so angle and angleSpread have no meaning there.</sch:assert>
      <sch:assert id="CRT16" test="(@crater and /scene//object3D[@primitive='voxels' and crater/@id=current()/@crater] and not(@count)) or (not(@crater and /scene//object3D[@primitive='voxels' and crater/@id=current()/@crater]) and @count)">a burst needs count, except one from the crater of an object of cells, whose particles are the cells that the cut throws and have no count.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-ocean">
    <sch:rule context="ocean">
      <sch:let name="cell" value="number(concat(substring(normalize-space(@cellSize),1+number(starts-with(normalize-space(@cellSize),'+'))),substring('0',1,number(not(@cellSize)))))+not(@cellSize)"/>
      <sch:let name="nx" value="(number(concat(substring(normalize-space(@width),1+number(starts-with(normalize-space(@width),'+'))),substring('0',1,number(not(@width)))))+64*not(@width)) div $cell"/>
      <sch:let name="nz" value="(number(concat(substring(normalize-space(@depth),1+number(starts-with(normalize-space(@depth),'+'))),substring('0',1,number(not(@depth)))))+64*not(@depth)) div $cell"/>
      <sch:let name="colliders" value="/scene//object3D[contains(concat(' ',normalize-space(current()/@colliders),' '),concat(' ',@id,' '))]"/>
      <sch:let name="count" value="count(str:tokenize(normalize-space(@colliders),' '))"/>
      <sch:assert id="OCN1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2')">ocean requires version="1.3" or later.</sch:assert>
      <sch:assert id="OCN2" test="(not(@bathymetry) or /scene/assets/image[@id=current()/@bathymetry and not(@layer)] or /scene/assets/mesh[@id=current()/@bathymetry]) and (not(@material) or /scene/materials/material[@id=current()/@material]) and (@bathymetry or not(@bathymetryScale|@bathymetryOffset|@bathymetryEncoding)) and (not(@bathymetryEncoding) or @bathymetryEncoding='red' or /scene/assets/image[@id=current()/@bathymetry])">ocean bathymetry must name a primary-raster image (no layer selection) or mesh, material must name a material, and bathymetry options require the corresponding asset kind.</sch:assert>
      <sch:assert id="OCN3" test="$nx&gt;=1 and $nz&gt;=1 and $nx - round($nx)&lt;=0.000000001*$nx and round($nx)-$nx&lt;=0.000000001*$nx and $nz - round($nz)&lt;=0.000000001*$nz and round($nz)-$nz&lt;=0.000000001*$nz and round($nx)*round($nz)&lt;=4000000 and (not(@dt) or number(substring(normalize-space(@dt),1+number(starts-with(normalize-space(@dt),'+'))))&gt;=0.000001) and not((@waterLevel|@bathymetryScale|@bathymetryOffset|@initialVelocityX|@initialVelocityZ|wave/@direction|wave/@phase|waterImpulse/@x|waterImpulse/@z|waterImpulse/@amplitude|waterImpulse/@velocityX|waterImpulse/@velocityZ)[not(number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+'))))-number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+'))))=0)]) and count(wave)&lt;=64 and count(waterImpulse)&lt;=16384 and not(wave[(number(concat(substring(normalize-space(@wavelength),1+number(starts-with(normalize-space(@wavelength),'+'))),substring('0',1,number(not(@wavelength)))))+16*not(@wavelength))&lt;2*$cell]) and not(waterImpulse[@type='add-water' and number(substring(normalize-space(@amplitude),1+number(starts-with(normalize-space(@amplitude),'+'))))&lt;0])">ocean requires finite inputs, integral grid dimensions (at most 4000000 cells), dt &gt;= 0.000001, resolved wavelengths, at most 64 waves/16384 impulses and nonnegative added water.</sch:assert>
      <sch:assert id="OCN4" test="not(*[@property and not(@property='x' or @property='y' or @property='z' or @property='rotation' or @property='rotationX' or @property='rotationY' or @property='scaleX' or @property='scaleY' or @property='scaleZ' or @property='opacity')])">ocean solver configuration is static; animate its pose or opacity instead.</sch:assert>
      <sch:assert id="OCN5" test="count(whitewater)&lt;=1 and not(whitewater[@end and number(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+')))) &lt; number(concat(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+'))),substring('0',1,number(not(@start)))))]) and not(whitewater/@foamMaterial[not(.=/scene/materials/material/@id)]|whitewater/@sprayMaterial[not(.=/scene/materials/material/@id)])">ocean accepts one whitewater source with end &gt;= start and valid foam/spray materials.</sch:assert>
      <sch:assert id="OCN6" test="not(@colliders) or ($count &lt;= 4096 and $count = count($colliders[(crater and (@primitive='plane' or @primitive='mesh')) or (not(crater) and (@primitive='box' or @primitive='sphere' or @primitive='globe' or @primitive='cylinder' or @primitive='cone' or @primitive='capsule' or @primitive='torus' or @primitive='mesh' or @primitive='text' or @primitive='extrude' or @primitive='clay'))]))">ocean colliders must name at most 4096 distinct objects: a plane or mesh with a crater, or a closed body without one.</sch:assert>
      <sch:assert id="OCN7" test="not($colliders/*[@property='primitive' or @property='mesh' or @property='radius' or @property='width' or @property='height' or @property='depth' or @property='segments' or (@property='exaggeration' and ../@primitive='globe' and ../@terrain) or (../@primitive='text' and (@property='text' or @property='font' or @property='bevel')) or (../@primitive='extrude' and (@property='path' or @property='bevel')) or (../@primitive='clay' and (@property='resolution' or @property='fingerprints' or @property='seed' or @property='boil'))]) and not($colliders[@primitive='clay']/blob/*[@property]) and not($colliders[@primitive='clay' and number(translate(@boil,'+',''))&gt;0 and number(translate(@fingerprints,'+',''))&gt;0])">ocean collider geometry is static; animate its position, rotation or scale instead of shape parameters.</sch:assert>
      <sch:assert id="OCN13" test="not(@splash) or (count(str:tokenize(normalize-space(@splash),' '))&gt;0 and count(str:tokenize(normalize-space(@splash),' '))=count(/scene//particles3D[burst[@crater] and contains(concat(' ',normalize-space(current()/@splash),' '),concat(' ',@id,' '))]))">ocean splash must name distinct particles3D emitters that throw out the ejecta of a crater, whose particles have a mass.</sch:assert>
      <sch:assert id="OCN14" test="not(whitewater/@foamRadius) or number(translate(whitewater/@foamRadius,'+','')) &lt;= 64 * number(translate(concat(@cellSize, substring('1', 1 div not(@cellSize))),'+',''))">ocean whitewater foamRadius is at most 64 cells of the ocean: a wider coverage takes too many distances to the vertices of the surface.</sch:assert>
      <sch:assert id="OCN8" test="not(@bodyCoupling) or @bodyCoupling='none' or @colliders">ocean bodyCoupling needs colliders that list the bodies.</sch:assert>
      <sch:assert id="OCN9" test="not(@bodyDrag) or @colliders">ocean bodyDrag belongs to an ocean with colliders.</sch:assert>
    </sch:rule>
    <sch:rule context="ocean/waterImpulse">
      <sch:assert id="OCN10" test="not(@source and (@time or @x or @z or @radius or @amplitude or @velocityX or @velocityZ or @type))">a water impulse from a body derives its instant, place, size and shape, so time, x, z, radius, amplitude, velocityX, velocityZ and type may not be given.</sch:assert>
      <sch:assert id="OCN11" test="not(@source) or (contains(concat(' ',normalize-space(../@colliders),' '),concat(' ',@source,' ')) and /scene//object3D[@id=current()/@source and not(crater)]/rigidBody[not(@type) or @type='dynamic'])">the source of a water impulse must be a dynamic rigid body, without a crater, that the ocean lists in colliders.</sch:assert>
      <sch:assert id="OCN12" test="not(@source) or count(../waterImpulse[@source=current()/@source])=1">a body makes one cavity: at most one water impulse names it.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-crater">
    <sch:rule context="object3D/crater">
      <sch:let name="radius" value="number(concat(substring(normalize-space(@radius),1+number(starts-with(normalize-space(@radius),'+'))),substring('50',1,2*number(not(@radius)))))"/>
      <sch:let name="depth" value="number(concat(substring(normalize-space(@depth),1+number(starts-with(normalize-space(@depth),'+'))),substring('10',1,2*number(not(@depth)))))"/>
      <sch:let name="rimWidth" value="number(concat(substring(normalize-space(@rimWidth),1+number(starts-with(normalize-space(@rimWidth),'+'))),substring('10',1,2*number(not(@rimWidth)))))"/>
      <sch:let name="rimHeight" value="number(concat(substring(normalize-space(@rimHeight),1+number(starts-with(normalize-space(@rimHeight),'+'))),substring('2',1,1*number(not(@rimHeight)))))"/>
      <sch:let name="influenceDepth" value="number(concat(substring(normalize-space(@influenceDepth),1+number(starts-with(normalize-space(@influenceDepth),'+'))),substring('0',1,1*number(not(@influenceDepth)))))"/>
      <sch:let name="normalX" value="number(concat(substring(normalize-space(@normalX),1+number(starts-with(normalize-space(@normalX),'+'))),substring('0',1,1*number(not(@normalX)))))"/>
      <sch:let name="normalY" value="number(concat(substring(normalize-space(@normalY),1+number(starts-with(normalize-space(@normalY),'+'))),substring('0',1,1*number(not(@normalY)))))"/>
      <sch:let name="normalZ" value="number(concat(substring(normalize-space(@normalZ),1+number(starts-with(normalize-space(@normalZ),'+'))),substring('-1',1,2*number(not(@normalZ)))))"/>
      <sch:let name="start" value="number(concat(substring(normalize-space(@start),1+number(starts-with(normalize-space(@start),'+'))),substring('0',1,1*number(not(@start)))))"/>
      <sch:let name="end" value="number(concat(substring(normalize-space(@end),1+number(starts-with(normalize-space(@end),'+'))),substring('1',1,1*number(not(@end)))))"/>
      <sch:assert id="CRT1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2')">crater deformation requires version="1.3" or later.</sch:assert>
      <sch:assert id="CRT2" test="parent::object3D[not(@primitive='volume')] and count(../crater)=1">one crater belongs to a surface object3D, not a volume.</sch:assert>
      <sch:assert id="CRT3" test="$end &gt; $start">crater end must be greater than start in the object's local clock.</sch:assert>
      <sch:assert id="CRT4" test="not(@*[name()!='curve' and name()!='id' and name()!='source' and name()!='capture' and name()!='mantle' and name()!='targetMaterial' and not((number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) - number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))))=0)]) and ($normalX!=0 or $normalY!=0 or $normalZ!=0) and $rimWidth&lt;=$radius and (not(@influenceDepth) or @source or ($influenceDepth*0.5 &gt;= $depth and $influenceDepth*0.5 &gt;= $rimHeight))">crater values must be finite, its normal nonzero, rimWidth &lt;= radius and, for a crater without a source, influenceDepth &gt;= twice max(depth,rimHeight); a crater with a source has its size from the impact, and its envelope is checked when it is made.</sch:assert>
      <sch:assert id="CRT6" test="not(@source and (@radius or @depth or @rimHeight or @rimWidth or @start or @end or @centerX or @centerY or @centerZ or @normalX or @normalY or @normalZ))">a crater that grows from a source derives its size, timing, centre and axis, so none of them may be given.</sch:assert>
      <sch:assert id="CRT7" test="(@source and @targetMaterial) or not(@source or @targetMaterial or @targetDensity or @strength or @gravity)">crater source requires targetMaterial, and targetMaterial, targetDensity, strength and gravity belong to a crater with a source.</sch:assert>
      <sch:assert id="CRT8" test="not(@source) or (/scene//object3D[@id=current()/@source and not(@id=current()/../@id)]/rigidBody[not(@type) or @type='dynamic'])">crater source must name another object3D whose rigidBody is dynamic.</sch:assert>
      <sch:assert id="CRT9" test="not(@capture) or @source">crater capture belongs to a crater that grows from a source.</sch:assert>
      <sch:assert id="CRT10" test="not(@mantle or @bulking or @repose) or @source">crater mantle, bulking and repose belong to a crater that grows from a source.</sch:assert>
      <sch:assert id="CRT11" test="not(@bulking) or normalize-space(@mantle)='true' or normalize-space(@mantle)='1'">crater bulking belongs to a crater with a mantle (mantle="true").</sch:assert>
      <sch:assert id="CRT12" test="not(@repose) or not(normalize-space(@mantle)='true' or normalize-space(@mantle)='1')">a crater that gives its settled ejecta a repose angle has no mantle: the ejecta are the ground once, as one or the other.</sch:assert>
      <sch:assert id="CRT5" test="not(../rigidBody[not(@type='static' or @type='kinematic') or (@shape and not(@shape='auto' or @shape='trimesh' or @shape='voxels'))])">crater rigid bodies require static/kinematic type with auto, trimesh or voxels collision geometry.</sch:assert>
      <sch:assert id="CRT13" test="not(parent::object3D[@primitive='voxels']) or not(@mantle or @bulking or @repose)">a crater in an object of cells has no mantle, bulking or repose: the rim is cells (bulking 1, a fifth heaped) and not an analytic surface.</sch:assert>
      <sch:assert id="CRT14" test="not(parent::object3D[@primitive='voxels']) or not(@curve)">the cut of a crater in an object of cells is instantaneous at the impact, so curve has no meaning there (start and end are refused by CRT6 for a crater with a source, which CRT15 requires).</sch:assert>
      <sch:assert id="CRT15" test="not(parent::object3D[@primitive='voxels']) or @source">a crater in an object of cells grows from a source: it is cut by an impact.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-black-hole-version">
    <sch:rule context="blackHole|accretionDisk|camera[@geodesics]">
      <sch:assert id="BH1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2')">black holes require version="1.3" or later.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-black-hole">
    <sch:rule context="blackHole">
      <sch:assert id="BH2" test="count(/scene//blackHole)=1">a scene has at most one blackHole.</sch:assert>
    </sch:rule>
    <sch:rule context="accretionDisk">
      <sch:let name="mass" value="number(substring(normalize-space(/scene//blackHole[@id=current()/@blackHole]/@mass),1+number(starts-with(normalize-space(/scene//blackHole[@id=current()/@blackHole]/@mass),'+'))))"/>
      <sch:let name="inner" value="number(substring(normalize-space(@innerRadius),1+number(starts-with(normalize-space(@innerRadius),'+'))))"/>
      <sch:let name="outer" value="number(substring(normalize-space(@outerRadius),1+number(starts-with(normalize-space(@outerRadius),'+'))))"/>
      <sch:assert id="BH3" test="/scene//blackHole[@id=current()/@blackHole]">an accretionDisk must name a blackHole.</sch:assert>
      <sch:assert id="BH4" test="not(/scene//blackHole[@id=current()/@blackHole]) or ((not(@innerRadius) or $inner &gt;= 6*$mass) and ((@innerRadius and $outer &gt; $inner) or (not(@innerRadius) and $outer &gt; 6*$mass)))">the inner radius of an accretionDisk is at least 6 times the mass of its blackHole (the innermost stable circular orbit), and the outer radius is beyond the inner one.</sch:assert>
    </sch:rule>
    <sch:rule context="camera[@geodesics]">
      <!-- SREP 76, open issue 4: geodesics is an xs:boolean, true in its forms "true" and "1", whitespace collapsed -->
      <sch:let name="geodesic" value="normalize-space(@geodesics)='true' or normalize-space(@geodesics)='1'"/>
      <!-- SREP 74: a viewport3D's subtree is a 3D scene of its own; BH5 to BH8 look only in the camera's scope (the
           viewport3D it is in, else the document outside every viewport3D) -->
      <sch:let name="scope" value="generate-id(ancestor::viewport3D[1])"/>
      <sch:let name="hole" value="(/scene//blackHole[generate-id(ancestor::viewport3D[1])=$scope])[1]"/>
      <sch:let name="mass" value="number(substring(normalize-space($hole/@mass),1+number(starts-with(normalize-space($hole/@mass),'+'))))"/>
      <sch:let name="dx" value="number(concat(substring(normalize-space(./@x),1+number(starts-with(normalize-space(./@x),'+'))),substring('0',1,1*number(not(./@x))))) - number(concat(substring(normalize-space($hole/@x),1+number(starts-with(normalize-space($hole/@x),'+'))),substring('0',1,1*number(not($hole/@x)))))"/>
      <sch:let name="dy" value="number(concat(substring(normalize-space(./@y),1+number(starts-with(normalize-space(./@y),'+'))),substring('0',1,1*number(not(./@y))))) - number(concat(substring(normalize-space($hole/@y),1+number(starts-with(normalize-space($hole/@y),'+'))),substring('0',1,1*number(not($hole/@y)))))"/>
      <sch:let name="dz" value="number(concat(substring(normalize-space(./@z),1+number(starts-with(normalize-space(./@z),'+'))),substring('0',1,1*number(not(./@z))))) - number(concat(substring(normalize-space($hole/@z),1+number(starts-with(normalize-space($hole/@z),'+'))),substring('0',1,1*number(not($hole/@z)))))"/>
      <sch:assert id="BH5" test="not($geodesic) or /scene//blackHole[generate-id(ancestor::viewport3D[1])=$scope]">a camera with geodesics="true" needs a blackHole.</sch:assert>
      <sch:assert id="BH6" test="not($geodesic) or not(/scene//*[self::object3D or self::particles3D or self::particleEmitter or self::ocean or self::fluid or self::flock or self::slime or self::erosion or self::pyro or self::medium][generate-id(ancestor::viewport3D[1])=$scope])">a camera with geodesics="true" renders only the black hole, its disk and the 2D layers: no object3D, particles3D, particleEmitter, ocean, fluid, flock, slime, erosion, pyro or medium may be in the scene.</sch:assert>
      <sch:assert id="BH7" test="not($geodesic) or not($hole) or $dx*$dx + $dy*$dy + $dz*$dz &gt; 9*$mass*$mass">a camera with geodesics="true" must be farther than 3 times the mass of the blackHole from it, the photon sphere.</sch:assert>
      <sch:assert id="BH8" test="not($geodesic) or count(/scene//camera[normalize-space(@geodesics)='true' or normalize-space(@geodesics)='1'][generate-id(ancestor::viewport3D[1])=$scope]) &lt;= 1">a scene has at most one camera with geodesics="true".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-fracture">
    <sch:rule context="fracture">
      <sch:assert id="FRX1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2')">fracture requires version="1.3" or later.</sch:assert>
      <sch:assert id="FRX2" test="parent::object3D[not(@primitive='volume' or @primitive='plane' or @primitive='map')] and count(../fracture)=1 and count(../rigidBody)=1">fracture requires one closed surface object3D owner and exactly one rigidBody.</sch:assert>
      <sch:assert id="FRX3" test="parent::object3D[@primitive='voxels'] or @interiorMaterial=/scene/materials/material/@id">fracture interiorMaterial must reference a declared material.</sch:assert>
      <sch:assert id="FRX4" test="not(@*[name()!='interiorMaterial' and name()!='source' and name()!='partition' and name()!='planes' and name()!='labels' and name()!='mode' and not((number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) - number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))))=0)])">fracture numeric values must be finite.</sch:assert>
      <sch:assert id="FRX5" test="not(@source) or (/scene//object3D[@id=current()/@source and not(@id=current()/../@id)]/rigidBody[not(@type) or @type='dynamic'])">fracture source must name another object3D whose rigidBody is dynamic.</sch:assert>
      <sch:assert id="FRX6" test="not(@source and (@at or @radialImpulse or @impulseX or @impulseY or @impulseZ))">a fracture that comes from a source derives its time and its push, so at, radialImpulse and impulseX, impulseY and impulseZ may not be given.</sch:assert>
      <sch:assert id="FRX7" test="@source or not(@minImpulse or @energyFraction)">minImpulse and energyFraction belong to a fracture with a source.</sch:assert>
      <sch:assert id="FRX8" test="not(parent::object3D[@primitive='voxels']) or not(@interiorMaterial or @interiorUvScale)">a fracture of an object of cells has no interior material: its pieces have the material of their cells.</sch:assert>
      <sch:assert id="FRX9" test="parent::object3D[@primitive='voxels'] or not(@partition or @planes or @labels)">partition, planes and labels belong to the fracture of an object of primitive voxels.</sch:assert>
      <sch:assert id="FRX10" test="not(@partition) or @partition='voronoi' or not(@pieces or @seed)">pieces and seed belong to a voronoi partition.</sch:assert>
      <sch:assert id="FRX11" test="(@partition='planes' or not(@planes)) and (not(@partition='planes') or (@planes and count(str:tokenize(normalize-space(@planes),' '))&gt;=4 and count(str:tokenize(normalize-space(@planes),' '))&lt;=252 and count(str:tokenize(normalize-space(@planes),' ')) mod 4 = 0 and count(str:tokenize(normalize-space(@planes),' ')[number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) - number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+')))) = 0]) = count(str:tokenize(normalize-space(@planes),' ')) and not(str:tokenize(normalize-space(@planes),' ')[position() mod 4 = 1 and number(substring(normalize-space(.),1+number(starts-with(normalize-space(.),'+'))))=0 and number(substring(normalize-space(following-sibling::*[1]),1+number(starts-with(normalize-space(following-sibling::*[1]),'+'))))=0 and number(substring(normalize-space(following-sibling::*[2]),1+number(starts-with(normalize-space(following-sibling::*[2]),'+'))))=0])))">partition planes takes planes, from one to 63 planes of four finite numbers (nx ny nz offset, the normal not all zeros), and planes belongs to that partition.</sch:assert>
      <sch:assert id="FRX13" test="(@mode='stress') = boolean(@strength)">fracture mode="stress" requires strength (pascals), and strength belongs to mode="stress".</sch:assert>
      <sch:assert id="FRX14" test="not(@mode='stress') or (parent::object3D[@primitive='voxels'] and ../rigidBody[not(@type) or @type='dynamic'])">fracture mode="stress" belongs to a dynamic body of cells (an object of primitive voxels with a dynamic rigidBody).</sch:assert>
      <sch:assert id="FRX15" test="not(@mode='stress') or not(@source or @at or @minImpulse or @energyFraction or @radialImpulse or @impulseX or @impulseY or @impulseZ)">a fracture by stress is fired by the load on the body and by nothing else, so source, at, minImpulse, energyFraction, radialImpulse and impulseX, impulseY and impulseZ may not be given.</sch:assert>
      <sch:assert id="FRX12" test="(@partition='labels' or not(@labels)) and (not(@partition='labels') or @labels='material')">partition labels takes labels="material", and labels belongs to that partition.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="cinematic-globe">
    <sch:rule context="object3D">
      <sch:assert id="GEO1" test="not(/scene/@version='1.0' or /scene/@version='1.1' or /scene/@version='1.2') or not((@terrain and @primitive='globe') or @planetRadius or @terrainTileSize or @terrainZoom or @terrainMissing or @terrainMemoryMiB)">globe elevation and its physical-scale/budget options require version="1.3" or later.</sch:assert>
      <sch:assert id="GEO2" test="(not(@planetRadius) or @primitive='globe') and (not(@terrainTileSize|@terrainZoom|@terrainMissing|@terrainMemoryMiB) or (@primitive='globe' and @terrain)) and (not(@terrainTileSize) or contains(' 1 2 4 8 16 32 64 128 256 512 1024 2048 4096 ', concat(' ',number(substring(normalize-space(@terrainTileSize),1+number(starts-with(normalize-space(@terrainTileSize),'+')))),' ')))">planetRadius belongs to globes; terrain sampling options require a globe with terrain and a power-of-two tile size.</sch:assert>
      <sch:assert id="GEO3" test="not(@primitive='globe' and *[@property='terrain' or @property='terrainEncoding' or @property='planetRadius' or @property='terrainTileSize' or @property='terrainZoom' or @property='terrainMissing' or @property='terrainMemoryMiB']) and not(@primitive='globe' and @terrain and rigidBody and *[@property='radius' or @property='segments' or @property='exaggeration'])">globe terrain sampling and planetRadius are static; globe rigid collider relief cannot animate radius, segments or exaggeration.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p77">
    <sch:rule context="@audioBuses | @audioTracks | @captions | @colliders | @duckUnder | @effects | @fit | @forceFields | @lights | @looks | @splash">
      <sch:assert id="R53" test="normalize-space(.) != ''">An ID reference list names at least one id.</sch:assert>
    </sch:rule>
    <sch:rule context="assets/audiogram">
      <sch:assert id="R54" test="/scene/audioMix/audioTrack[@id = current()/@source]">audiogram/@source names an audioTrack of audioMix.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep67">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5']">
      <sch:assert id="V13" test="not(.//compute or .//iterate or effects/effect[@type='error-diffusion' or @type='segmented-sort'])">compute, iterate, error-diffusion and segmented-sort need version="1.6".</sch:assert>
    </sch:rule>
    <sch:rule context="iterate">
      <sch:assert id="ITR1" test="number(substring(normalize-space(@checkEvery),1+number(starts-with(normalize-space(@checkEvery),'+')))) &lt;= number(substring(normalize-space(@steps),1+number(starts-with(normalize-space(@steps),'+')))) or not(@checkEvery)">iterate/@checkEvery is at most @steps.</sch:assert>
      <sch:assert id="ITR2" test="not(.//iterate)">iterate does not nest.</sch:assert>
    </sch:rule>
    <sch:rule context="compute">
      <sch:assert id="CMP13" test="count(tonemap) &lt;= 1">a compute node has at most one tonemap.</sch:assert>
    </sch:rule>
    <sch:rule context="effects/effect[@type='segmented-sort']">
      <sch:assert id="SRT1" test="number(substring(normalize-space(@low),1+number(starts-with(normalize-space(@low),'+')))) &lt;= number(substring(normalize-space(@high),1+number(starts-with(normalize-space(@high),'+')))) or not(@low) or not(@high)">segmented-sort needs low &lt;= high.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <!-- ===================================================== programs (SREPs 66 and 69, scene version 1.6) -->
  <sch:pattern id="p-srep66">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5']">
      <sch:assert id="V12" test="not(.//program)">program needs version="1.6".</sch:assert>
    </sch:rule>
    <sch:rule context="program/param">
      <sch:assert id="PRG1" test="not(preceding-sibling::param[@name = current()/@name])">a program's parameter names are unique.</sch:assert>
    </sch:rule>
    <sch:rule context="repeat[@over]">
      <sch:assert id="PRG2" test="not(//program[@id = current()/@over]) or /scene/parameters/program[@id = current()/@over]">repeat/@over names a data program (one in parameters), not a node program.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep69">
    <sch:rule context="program[@mode='step']">
      <sch:assert id="PRG3" test="@width and @height">a stepping program needs width and height.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep68">
    <sch:rule context="effects/effect[@stepsPerFrame or @prewarm] | program[@stepsPerFrame or @prewarm]">
      <sch:assert id="STP1" test="(self::effect and @type='shader') or (self::program and @mode='step')">stepsPerFrame and prewarm apply to shader effects and stepping programs (SREP 69) only.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep70">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5']">
      <sch:assert id="V14" test="not(.//shape[@shape='parametric'] or .//object3D[@primitive='parametric' or @primitive='heightfield'])">parametric shapes, parametric surfaces and heightfields need version="1.6".</sch:assert>
    </sch:rule>
    <sch:rule context="shape">
      <sch:assert id="PAR1" test="(@shape='parametric') = (count(parametricPath) = 1)">a shape with shape="parametric" has exactly one parametricPath, and only such a shape has one.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D">
      <sch:assert id="PAR2" test="(@primitive='parametric') = (count(parametricSurface) = 1) and (@primitive='heightfield') = (count(heightfield) = 1)">primitive="parametric" has exactly one parametricSurface and primitive="heightfield" exactly one heightfield; no other primitive has either.</sch:assert>
      <sch:assert id="PLT3" test="(@primitive='axes') = (count(axes3D) = 1) and (not(plot3D) or @primitive='axes')">primitive="axes" has exactly one axes3D, and only such an object has an axes3D or a plot3D.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71">
    <sch:rule context="lights/light[@sky and @sky != 'none']">
      <sch:assert id="SKY1" test="@type='dome'">sky applies to dome lights only.</sch:assert>
      <sch:assert id="SKY2" test="not(@environment)">a dome has either an environment image or a procedural sky, not both.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71-baseColorMap">
    <sch:rule context="material[starts-with(@baseColorMap, '#')]">
      <sch:assert id="MTX1-baseColorMap" test="/scene/assets/generator[@id = substring(current()/@baseColorMap, 2)] or /scene/assets/image[@id = substring(current()/@baseColorMap, 2)]">a material map that starts with # names a generator or image asset of the document.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71-normalMap">
    <sch:rule context="material[starts-with(@normalMap, '#')]">
      <sch:assert id="MTX1-normalMap" test="/scene/assets/generator[@id = substring(current()/@normalMap, 2)] or /scene/assets/image[@id = substring(current()/@normalMap, 2)]">a material map that starts with # names a generator or image asset of the document.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71-metallicRoughnessMap">
    <sch:rule context="material[starts-with(@metallicRoughnessMap, '#')]">
      <sch:assert id="MTX1-metallicRoughnessMap" test="/scene/assets/generator[@id = substring(current()/@metallicRoughnessMap, 2)] or /scene/assets/image[@id = substring(current()/@metallicRoughnessMap, 2)]">a material map that starts with # names a generator or image asset of the document.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71-occlusionMap">
    <sch:rule context="material[starts-with(@occlusionMap, '#')]">
      <sch:assert id="MTX1-occlusionMap" test="/scene/assets/generator[@id = substring(current()/@occlusionMap, 2)] or /scene/assets/image[@id = substring(current()/@occlusionMap, 2)]">a material map that starts with # names a generator or image asset of the document.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71-emissiveMap">
    <sch:rule context="material[starts-with(@emissiveMap, '#')]">
      <sch:assert id="MTX1-emissiveMap" test="/scene/assets/generator[@id = substring(current()/@emissiveMap, 2)] or /scene/assets/image[@id = substring(current()/@emissiveMap, 2)]">a material map that starts with # names a generator or image asset of the document.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep71-displacementMap">
    <sch:rule context="material[starts-with(@displacementMap, '#')]">
      <sch:assert id="MTX1-displacementMap" test="/scene/assets/generator[@id = substring(current()/@displacementMap, 2)] or /scene/assets/image[@id = substring(current()/@displacementMap, 2)]">a material map that starts with # names a generator or image asset of the document.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep74">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5']">
      <sch:assert id="V15" test="not(.//viewport3D)">viewport3D needs version="1.6".</sch:assert>
    </sch:rule>
    <sch:rule context="viewport3D">
      <sch:assert id="VP1" test="not(@camera) or camera[@id = current()/@camera]">viewport3D/@camera names a camera child of that viewport.</sch:assert>
      <sch:assert id="VP3" test="not(.//rigidBody)">bodies inside a viewport3D are not supported (no physics world of its own yet).</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep75">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5']">
      <sch:assert id="V16" test="not(assets/fractal)">fractal assets need version="1.6".</sch:assert>
    </sch:rule>
    <sch:rule context="assets/fractal">
      <sch:assert id="FRC1" test="(@kind='julia') = (boolean(@juliaX) and boolean(@juliaY))">a julia fractal has juliaX and juliaY; a mandelbrot has neither.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-v17">
    <sch:rule context="/scene[@version='1.0' or @version='1.1' or @version='1.2' or @version='1.3' or @version='1.4' or @version='1.5' or @version='1.6' or @version='1.7']">
      <sch:let name="partTag" value="'\part{'"/>
      <sch:assert id="V17" test="not(.//axes or .//plot or .//object3D[@primitive='axes'] or .//axes3D or .//plot3D or .//modifier[@type='matrix' or @type='map' or @type='complex'] or .//group/deform or .//formulaAnimator or assets/formulaMorph or .//shape[@morph] or assets/formula[contains(@tex, $partTag)] or assets/value or .//field or space or assets/ephemeris or .//object3D[@body or @positionFrom or @orientation] or assets/catalogue or .//light/starField or .//light[@sky='physical'] or .//object3D/orbit or .//object3D/trail or space/nbody or .//object3D[@primitive='planet' or @flattening or @nightLights or @twilight or @cloudMap or @cloudHeight or @cloudRotation or @cloudShadow or @ringMap or @ringInner or @ringOuter or @limbDarkening or @atmosphere or @atmosphereHeight] or .//camera[@depth or scaleBar] or .//key[@interpolation='geometric'] or .//motionPath[@interpolation='geometric'] or .//animate[@defaultInterpolation='geometric'] or .//timeRemap[@defaultInterpolation='geometric'] or .//flyTo[@easing='geometric'] or .//transition[@curve='geometric'] or .//particleEmitter[@sizeCurve='geometric' or @colorCurve='geometric'] or .//particles3D[@sizeCurve='geometric' or @colorCurve='geometric'])">axes, plot, axes3D, plot3D, primitive="axes", the modifier types matrix, map and complex, a group's deform, formulaAnimator, formulaMorph, shape/@morph, \part in a formula, value assets, field nodes, space, ephemeris assets, object3D/@body, @positionFrom and @orientation, catalogue assets, starField, sky="physical", primitive="planet" and its attributes, orbit, nbody and trail, camera/@depth, scaleBar and the geometric curve need version="1.8".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-plots">
    <sch:rule context="plot">
      <sch:assert id="PLT1" test="(@kind='explicit' and @y and not(@x or @r or @f)) or (@kind='parametric' and @x and @y and not(@r or @f)) or (@kind='polar' and @r and not(@x or @y or @f)) or (@kind='implicit' and @f and not(@x or @y or @r))">a plot has the expressions of its kind: explicit y; parametric x and y; polar r; implicit f; and no other.</sch:assert>
    </sch:rule>
    <sch:rule context="axes | axes3D">
      <sch:assert id="PLT2" test="(sum(@xMin) - 1*number(not(@xMin))) &lt; (sum(@xMax) + 1*number(not(@xMax))) and (sum(@yMin) - 1*number(not(@yMin))) &lt; (sum(@yMax) + 1*number(not(@yMax))) and (not(self::axes3D) or (sum(@zMin) - 1*number(not(@zMin))) &lt; (sum(@zMax) + 1*number(not(@zMax))))">each min of an axes or axes3D is below its max.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-semantic-tex">
    <sch:rule context="assets/formulaMorph">
      <sch:assert id="FRM2" test="/scene/assets/formula[@id=current()/@from] and /scene/assets/formula[@id=current()/@to]">formulaMorph/@from and @to name formula assets.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-reactive">
    <sch:rule context="assets/value">
      <sch:assert id="RV1" test="boolean(@update) = (boolean(@step) or boolean(@count)) and not(@step and @count)">a value has update with exactly one of step and count.</sch:assert>
      <sch:assert id="RV2" test="not(@between = 'linear') or boolean(@step)">between applies to a time fold.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-fields">
    <sch:rule context="field">
      <sch:assert id="FLD1" test="count(@u | @function | @w | @grid | @forceField) + number(boolean(charge | current | dipole | wire)) = 1 and (not(@v) or (@u and (not(@kind) or @kind='vector'))) and ((@kind='scalar' and ((@u and not(@v)) or @grid)) or (@kind='complex' and (@function or @w)) or ((not(@kind) or @kind='vector') and ((@u and @v) or @grid or @forceField or charge or current or dipole or wire))) and (not(@coefficients) or @function='mobius') and (not(@function='mobius') or count(str:tokenize(normalize-space(@coefficients), ' '))=8)">a field has exactly one source, fitting its kind: a vector field u with v, grid, forceField, or charge, current, dipole and wire children; a scalar field u alone or grid; a complex field function or w; coefficients go with function="mobius" only, eight of them.</sch:assert>
      <sch:assert id="FLD2" test="(not(@xMin and @xMax) or number(@xMin) &lt; number(@xMax)) and (not(@yMin and @yMax) or number(@yMin) &lt; number(@yMax))">each given min of a field is below its max.</sch:assert>
      <sch:assert id="FLD5" test="ancestor::axes or (@width and @height)">a field outside an axes states its box: width and height.</sch:assert>
    </sch:rule>
    <sch:rule context="field/portrait | field/contours | field/lines | field/streamlines">
      <sch:assert id="FLD3" test="(not(self::portrait) or ../@kind='complex') and (not(self::contours) or ../@kind='scalar') and (not(self::lines) or ../charge)">a portrait needs a complex field, contours a scalar field, and lines a field with charges.</sch:assert>
      <sch:assert id="FLD4" test="(not(@test) or number(@test) &lt;= sum(@separation) + 20*number(not(@separation))) and not(@maxSteps &gt; 1000000) and not(@maxLines &gt; 1000000)">a streamlines test is at most its separation, and maxSteps and maxLines are at most 1000000.</sch:assert>
    </sch:rule>
    <sch:rule context="field/arrows[@show='poynting']">
      <sch:assert id="FLD6" test="../wire[@i != 0] and (../charge or ../dipole or ../wire[@q != 0])">Poynting arrows need a current-carrying wire and a charge.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-time-frames">
    <sch:rule context="/scene/space">
      <sch:let name="e" value="string(@epoch)"/>
      <sch:let name="approx" value="not(@ephemeris) or @ephemeris='approximate'"/>
      <sch:let name="zlen" value="6 - 5*number(substring($e,string-length($e))='Z')"/>
      <sch:let name="stamp" value="substring($e,1,string-length($e)-$zlen)"/>
      <sch:let name="zone" value="substring($e,string-length($e)-$zlen+1)"/>
      <sch:let name="Y" value="number(substring($e,1,4))"/>
      <sch:let name="M" value="number(substring($e,6,2))"/>
      <sch:let name="D" value="number(substring($e,9,2))"/>
      <sch:let name="h" value="number(substring($e,12,2))"/>
      <sch:let name="mi" value="number(substring($e,15,2))"/>
      <sch:let name="leap" value="($Y mod 4 = 0 and $Y mod 100 != 0) or $Y mod 400 = 0"/>
      <sch:let name="days" value="number(substring('312831303130313130313031',2*$M - 1,2)) + number($M=2 and $leap)"/>
      <sch:let name="shift" value="(number(substring($zone,1,1)='+') - number(substring($zone,1,1)='-'))*(60*number(concat('0',substring($zone,2,2)))+number(concat('0',substring($zone,5,2))))"/>
      <sch:let name="u" value="60*$h + $mi - $shift"/>
      <sch:let name="uy" value="$Y + number($u &gt;= 1440 and $M=12 and $D=31) - number($u &lt; 0 and $M=1 and $D=1)"/>
      <sch:let name="utc" value="string-length($e) &gt;= 20 and translate(substring($stamp,1,19),'0123456789','0000000000')='0000-00-00T00:00:00' and (string-length($stamp)=19 or (substring($stamp,20,1)='.' and string-length($stamp) &gt; 20 and translate(substring($stamp,21),'0123456789','')='')) and ($zone='Z' or translate($zone,'0123456789+','0000000000-')='-00:00') and $M &gt;= 1 and $M &lt;= 12 and $D &gt;= 1 and $D &lt;= $days and $h &lt;= 23 and $mi &lt;= 59 and number(substring($e,18,2)) &lt;= 59 and ($zone='Z' or (number(substring($zone,2,2)) &lt;= 23 and number(substring($zone,5,2)) &lt;= 59)) and $uy &gt;= 1972 and (not($approx) or $uy &lt;= 3000)"/>
      <sch:let name="j" value="normalize-space(@epoch)"/>
      <sch:let name="digits" value="substring($j,1+number(starts-with($j,'+') or starts-with($j,'-')))"/>
      <sch:let name="jd" value="number(substring(normalize-space(@epoch),1+number(starts-with(normalize-space(@epoch),'+'))))"/>
      <sch:let name="julian" value="$digits != '' and $digits != '.' and translate($digits,'0123456789.','')='' and string-length($digits) - string-length(translate($digits,'.','')) &lt;= 1 and (not($approx) or ($jd &gt;= 625332.5 and $jd &lt; 2817152.5))"/>
      <sch:let name="b" value="number(substring(normalize-space(@frameBody),1+number(starts-with(normalize-space(@frameBody),'+'))))"/>
      <sch:let name="o" value="number(substring(normalize-space(@origin),1+number(starts-with(normalize-space(@origin),'+'))))"/>
      <sch:assert id="TF2" test="(@timeScale='tt' or @timeScale='tdb') and $julian or not(@timeScale='tt' or @timeScale='tdb') and $utc">a space's epoch is YYYY-MM-DDThh:mm:ss[.s] with Z or a ±hh:mm offset from 1972-01-01 on (timeScale="utc") or a Julian day (tt, tdb); with the approximate elements its year is -3000 to 3000.</sch:assert>
      <sch:assert id="TF3" test="not(@frame='bodyFixed') or ($b=10 or $b=199 or $b=299 or $b=399 or $b=499 or $b=599 or $b=699 or $b=799 or $b=899 or $b=301)">frame="bodyFixed" needs a frameBody with rotation elements: 10, 199, 299, 399, 499, 599, 699, 799, 899 or 301.</sch:assert>
      <sch:assert id="TF5" test="(not(@rate) or number(substring(normalize-space(@rate),1+number(starts-with(normalize-space(@rate),'+')))) = number(substring(normalize-space(@rate),1+number(starts-with(normalize-space(@rate),'+'))))) and ($approx or /scene/assets/ephemeris[@id=current()/@ephemeris]) and (not($approx) or not(@origin) or $o=0 or $o=10 or ($o &gt;= 1 and $o &lt;= 8))">a space's rate is finite; its ephemeris is approximate or names an ephemeris asset; with the approximate elements the origin is 0, 10 or a planet 1 to 8.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D[@positionFrom='ephemeris' or @orientation='bodyFixed']">
      <sch:let name="b" value="number(substring(normalize-space(@body),1+number(starts-with(normalize-space(@body),'+'))))"/>
      <sch:let name="approx" value="not(/scene/space/@ephemeris) or /scene/space/@ephemeris='approximate'"/>
      <sch:assert id="TF4" test="@body and /scene/space and (not(@positionFrom='ephemeris') or not($approx) or ($b &gt;= 1 and $b &lt;= 8)) and (not(@orientation='bodyFixed') or $b=10 or $b=199 or $b=299 or $b=399 or $b=499 or $b=599 or $b=699 or $b=799 or $b=899 or $b=301)">positionFrom="ephemeris" and orientation="bodyFixed" need a body and a space; with the approximate elements the body is a planet 1 to 8, and orientation="bodyFixed" a body with rotation elements.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-star-fields">
    <sch:rule context="light/starField">
      <sch:let name="limit" value="number(substring(normalize-space(@limitMagnitude),1+number(starts-with(normalize-space(@limitMagnitude),'+'))))"/>
      <sch:let name="epoch" value="number(substring(normalize-space(@epoch),1+number(starts-with(normalize-space(@epoch),'+'))))"/>
      <sch:assert id="SF2" test="../@type='dome' and count(../starField)=1 and /scene/assets/catalogue[@id=current()/@catalogue]">starField is written once, on a dome light, and names a catalogue asset.</sch:assert>
      <sch:assert id="SF3" test="not(@limitMagnitude) or $limit = $limit">a star field's limitMagnitude is finite.</sch:assert>
      <sch:assert id="SF4" test="not(@epoch) or ($epoch &gt;= -8000 and $epoch &lt;= 12000)">a star field's epoch is within 10000 years of 2000.0.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-physical-sky">
    <sch:rule context="lights/light[@sky='physical']">
      <sch:let name="neg" value="starts-with(@date,'-')"/>
      <sch:let name="ds" value="substring(@date,1+number($neg))"/>
      <sch:let name="shape" value="translate($ds,'123456789','000000000')"/>
      <sch:let name="tail" value="substring($shape,20)"/>
      <sch:let name="tn" value="string-length($tail)"/>
      <sch:let name="zl" value="1+5*number(substring($tail,$tn)!='Z')"/>
      <sch:let name="zone" value="substring($tail,$tn - $zl + 1)"/>
      <sch:let name="frac" value="substring($tail,1,$tn - $zl)"/>
      <sch:let name="year" value="number(substring($ds,1,4))*(1-2*number($neg))"/>
      <sch:let name="lat" value="number(substring(normalize-space(@latitude),1+number(starts-with(normalize-space(@latitude),'+'))))"/>
      <sch:let name="lon" value="number(substring(normalize-space(@longitude),1+number(starts-with(normalize-space(@longitude),'+'))))"/>
      <sch:let name="temp" value="number(substring(normalize-space(@temperature),1+number(starts-with(normalize-space(@temperature),'+'))))"/>
      <sch:let name="dt" value="number(substring(normalize-space(@deltaT),1+number(starts-with(normalize-space(@deltaT),'+'))))"/>
      <sch:let name="albedo" value="number(substring(normalize-space(@groundAlbedo),1+number(starts-with(normalize-space(@groundAlbedo),'+'))))"/>
      <sch:assert id="PSKY2" test="@date and substring($shape,1,19)='0000-00-00T00:00:00' and ($zone='Z' or $zone='+00:00' or $zone='-00:00') and ($frac='' or (starts-with($frac,'.') and string-length($frac)&gt;=2 and translate(substring($frac,2),'0','')='')) and number(substring($ds,6,2))&gt;=1 and number(substring($ds,6,2))&lt;=12 and number(substring($ds,9,2))&gt;=1 and number(substring($ds,9,2))&lt;=31 and number(substring($ds,12,2))&lt;=23 and number(substring($ds,15,2))&lt;=59 and number(substring($ds,18,2))&lt;=60 and $year&gt;=-2000 and $year&lt;=6000 and (($year&gt;=1900 and $year&lt;=2100) or @deltaT)">a physical sky's date is YYYY-MM-DDThh:mm:ss[.f] with Z or a +hh:mm offset, its year -2000 to 6000, and deltaT is given for years outside 1900 to 2100.</sch:assert>
      <sch:assert id="PSKY3" test="(not(@latitude) or ($lat&gt;=-90 and $lat&lt;=90)) and (not(@longitude) or ($lon&gt;=-180 and $lon&lt;=180)) and (not(@temperature) or $temp&gt;-273) and (not(@deltaT) or ($dt&gt;=-8000 and $dt&lt;=8000)) and (not(@groundAlbedo) or ($albedo&gt;=0 and $albedo&lt;=1))">a physical sky's latitude is -90 to 90, longitude -180 to 180, temperature above -273, deltaT -8000 to 8000 and groundAlbedo 0 to 1.</sch:assert>
      <sch:assert id="PSKY4" test="not(@sunAzimuth) and not(@sunElevation)">a physical sky's sun comes from its date: sunAzimuth and sunElevation are not written.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-planets">
    <sch:rule context="object3D[@primitive='planet']">
      <sch:let name="b" value="number(substring(normalize-space(@body),1+number(starts-with(normalize-space(@body),'+'))))"/>
      <sch:let name="iau" value="$b=10 or $b=199 or $b=299 or $b=399 or $b=499 or $b=599 or $b=699 or $b=799 or $b=899 or $b=301"/>
      <sch:let name="inner" value="number(substring(concat(normalize-space(@ringInner),substring('1.239',1,99*number(not(@ringInner)))),1+number(starts-with(concat(normalize-space(@ringInner),substring('1.239',1,99*number(not(@ringInner)))),'+'))))"/>
      <sch:let name="outer" value="number(substring(concat(normalize-space(@ringOuter),substring('2.269',1,99*number(not(@ringOuter)))),1+number(starts-with(concat(normalize-space(@ringOuter),substring('2.269',1,99*number(not(@ringOuter)))),'+'))))"/>
      <sch:assert id="PL1" test="not(@body) or $iau or @radius">a planet's body is one with IAU radii (10, 199, 299, 399, 499, 599, 699, 799, 899 or 301), else the planet has a radius.</sch:assert>
      <sch:assert id="PL3" test="(not(@flattening) or number(substring(normalize-space(@flattening),1+number(starts-with(normalize-space(@flattening),'+')))) &lt; 1) and (not(@twilight) or number(substring(normalize-space(@twilight),1+number(starts-with(normalize-space(@twilight),'+')))) &lt;= 90) and (not(@cloudHeight) or number(substring(normalize-space(@cloudHeight),1+number(starts-with(normalize-space(@cloudHeight),'+')))) &lt;= 1) and (not(@ringInner or @ringOuter) or ($inner &gt; 1 and $outer &gt; $inner)) and (not(@atmosphereHeight) or number(substring(normalize-space(@atmosphereHeight),1+number(starts-with(normalize-space(@atmosphereHeight),'+')))) &gt; 0)">a planet's flattening is below 1, twilight at most 90, cloudHeight at most 1, ringInner above 1 and ringOuter above ringInner, atmosphereHeight positive.</sch:assert>
      <sch:assert id="PL4" test="(@cloudMap or not(@cloudHeight or @cloudRotation or @cloudShadow)) and (@ringMap or not(@ringInner or @ringOuter))">cloudHeight, cloudRotation and cloudShadow are written only with cloudMap; ringInner and ringOuter only with ringMap.</sch:assert>
      <sch:assert id="PL5" test="not(@limbDarkening='solar') or (//material[@id=current()/@material][@emissiveMap or @emissive] and (not(@body) or $b=10))">limbDarkening="solar" needs a material with emissiveMap or emissive and is written with body="10" or without body.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D">
      <sch:assert id="PL2" test="not(@flattening or @nightLights or @twilight or @cloudMap or @cloudHeight or @cloudRotation or @cloudShadow or @ringMap or @ringInner or @ringOuter or @limbDarkening or @atmosphere or @atmosphereHeight)">flattening, nightLights, twilight, cloudMap, cloudHeight, cloudRotation, cloudShadow, ringMap, ringInner, ringOuter, limbDarkening, atmosphere and atmosphereHeight are written only with primitive="planet".</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-orbits">
    <sch:rule context="object3D/orbit">
      <sch:let name="e" value="number(substring(concat(normalize-space(@e),substring('0',1,99*number(not(@e)))),1+number(starts-with(concat(normalize-space(@e),substring('0',1,99*number(not(@e)))),'+'))))"/>
      <sch:let name="a" value="number(substring(normalize-space(@a),1+number(starts-with(normalize-space(@a),'+'))))"/>
      <sch:let name="i" value="number(substring(concat(normalize-space(@i),substring('0',1,99*number(not(@i)))),1+number(starts-with(concat(normalize-space(@i),substring('0',1,99*number(not(@i)))),'+'))))"/>
      <sch:let name="raan" value="number(substring(concat(normalize-space(@raan),substring('0',1,99*number(not(@raan)))),1+number(starts-with(concat(normalize-space(@raan),substring('0',1,99*number(not(@raan)))),'+'))))"/>
      <sch:let name="argp" value="number(substring(concat(normalize-space(@argp),substring('0',1,99*number(not(@argp)))),1+number(starts-with(concat(normalize-space(@argp),substring('0',1,99*number(not(@argp)))),'+'))))"/>
      <sch:let name="nu" value="number(substring(concat(normalize-space(@anomaly),substring('0',1,99*number(not(@anomaly)))),1+number(starts-with(concat(normalize-space(@anomaly),substring('0',1,99*number(not(@anomaly)))),'+'))))"/>
      <sch:let name="epoch" value="number(substring(concat(normalize-space(@epoch),substring('0',1,99*number(not(@epoch)))),1+number(starts-with(concat(normalize-space(@epoch),substring('0',1,99*number(not(@epoch)))),'+'))))"/>
      <sch:assert id="OR1" test="//object3D[@id=current()/@around][@id!=current()/../@id] and (@mu or /scene/space/nbody/member[@ref=current()/@around][@mass or contains(' 10 199 299 399 499 599 699 799 899 301 ',concat(' ',number(substring(normalize-space(@body),1+number(starts-with(normalize-space(@body),'+')))),' '))] or //object3D[@id=current()/@around][contains(' 10 199 299 399 499 599 699 799 899 301 ',concat(' ',number(substring(normalize-space(@body),1+number(starts-with(normalize-space(@body),'+')))),' '))])">an orbit's around names another object3D, and the orbit has a gravitational parameter: mu, or a central object that is an nbody member with a mass or has a body with a GM (10, 199, 299, 399, 499, 599, 699, 799, 899 or 301).</sch:assert>
      <sch:assert id="OR2" test="($e - 1 &gt;= 1e-12 or 1 - $e &gt;= 1e-12) and (($e &lt; 1 and $a &gt; 0) or ($e &gt; 1 and $a &lt; 0)) and ($e &lt; 1 or 1 + $e * math:cos($nu * 3.141592653589793 div 180) &gt; 0) and $i=$i and $raan=$raan and $argp=$argp and $nu=$nu and $epoch=$epoch">an orbit's e differs from 1 by at least 1e-12; a is positive when e is below 1 and negative when above, and with e above 1, 1 + e cos(anomaly) is positive; the angles and the epoch are finite.</sch:assert>
    </sch:rule>
    <sch:rule context="/scene/space/nbody">
      <sch:let name="fps" value="number(substring-before(concat(/scene/project/@fps,'/'),'/')) div number(concat(substring-after(/scene/project/@fps,'/'),substring('1',1,number(not(contains(/scene/project/@fps,'/'))))))"/>
      <sch:let name="rate" value="number(substring(concat(normalize-space(../@rate),substring('1',1,99*number(not(../@rate)))),1+number(starts-with(concat(normalize-space(../@rate),substring('1',1,99*number(not(../@rate)))),'+'))))"/>
      <sch:let name="k" value="$rate div $fps div number(substring(concat(normalize-space(@step),substring('3600',1,99*number(not(@step)))),1+number(starts-with(concat(normalize-space(@step),substring('3600',1,99*number(not(@step)))),'+'))))"/>
      <sch:assert id="OR3" test="not(member[@ref=preceding-sibling::member/@ref]) and count(member)=count(member[@ref=//object3D/@id]) and not(member[not(@mass) and not(contains(' 10 199 299 399 499 599 699 799 899 301 ',concat(' ',number(substring(normalize-space(@body),1+number(starts-with(normalize-space(@body),'+')))),' ')))]) and (@tolerance or ($k - round($k) &lt;= 1e-9 and round($k) - $k &lt;= 1e-9)) and (not(@tolerance) or @integrator='ias15') and (not(@integrator='wisdomHolman') or not(@softening) or number(substring(normalize-space(@softening),1+number(starts-with(normalize-space(@softening),'+'))))=0)">an nbody's members name distinct object3Ds, each with a mass or a body with a GM; without tolerance the frame interval rate / fps is a whole number of steps within 1e-9 of a step; tolerance is written only with integrator="ias15"; softening is 0 with wisdomHolman.</sch:assert>
    </sch:rule>
    <sch:rule context="object3D">
      <sch:assert id="OR4" test="count(orbit) + count(/scene/space/nbody/member[@ref=current()/@id]) + number(@positionFrom='ephemeris') &lt;= 1">an object has at most one of an orbit, an nbody member entry and positionFrom="ephemeris".</sch:assert>
      <sch:assert id="OR5" test="not(trail) or (count(trail)=1 and (orbit or /scene/space/nbody/member[@ref=current()/@id]) and not(trail[number(substring(normalize-space(@samples),1+number(starts-with(normalize-space(@samples),'+')))) &lt; 2]))">a trail is written once, on an object with an orbit or an nbody member entry, and has at least 2 samples.</sch:assert>
    </sch:rule>
  </sch:pattern>
  <sch:pattern id="p-srep-scale">
    <sch:rule context="camera">
      <sch:let name="near" value="number(substring(concat(normalize-space(@near),substring('0.1',1,99*number(not(@near)))),1+number(starts-with(concat(normalize-space(@near),substring('0.1',1,99*number(not(@near)))),'+'))))"/>
      <sch:let name="far" value="number(substring(concat(normalize-space(@far),substring('10000',1,99*number(not(@far)))),1+number(starts-with(concat(normalize-space(@far),substring('10000',1,99*number(not(@far)))),'+'))))"/>
      <sch:assert id="SC1" test="$near &gt; 0 and $far &gt; $near and $far div $near &lt;= 100000000000000000000">a camera's near is positive, its far exceeds its near, and far divided by near is at most 1e20.</sch:assert>
      <sch:assert id="SC3" test="count(scaleBar) &lt;= 1 and not(scaleBar[number(substring(concat(normalize-space(@maxWidth),substring('200',1,99*number(not(@maxWidth)))),1+number(starts-with(concat(normalize-space(@maxWidth),substring('200',1,99*number(not(@maxWidth)))),'+')))) &gt; number(substring(normalize-space(/scene/project/@width),1+number(starts-with(normalize-space(/scene/project/@width),'+'))))]) and not(scaleBar[@units and @units!='auto' and @units!='si'])">a camera has at most one scaleBar, whose maxWidth is at most the scene width and whose units are auto or si.</sch:assert>
      <sch:assert id="SC4" test="not(scaleBar[@distance][not(number(substring(normalize-space(@distance),1+number(starts-with(normalize-space(@distance),'+')))) &gt;= $near and number(substring(normalize-space(@distance),1+number(starts-with(normalize-space(@distance),'+')))) &lt;= $far)])">a scaleBar's distance lies between its camera's near and far.</sch:assert>
    </sch:rule>
    <sch:rule context="key[following-sibling::key][@interpolation='geometric' or (not(@interpolation) and ../@defaultInterpolation='geometric')] | motionPath | flyTo | transition | particleEmitter | particles3D">
      <sch:let name="v0" value="number(substring(normalize-space(@value),1+number(starts-with(normalize-space(@value),'+'))))"/>
      <sch:let name="v1" value="number(substring(normalize-space(following-sibling::key[1]/@value),1+number(starts-with(normalize-space(following-sibling::key[1]/@value),'+'))))"/>
      <sch:assert id="SC2" test="(self::key and (not($v0 * 0 = 0 and $v1 * 0 = 0) or ($v0 &gt; 0 and $v1 &gt; 0) or ($v0 &lt; 0 and $v1 &lt; 0))) or (not(self::key) and not(@interpolation='geometric' or @easing='geometric' or @curve='geometric' or @sizeCurve='geometric' or @colorCurve='geometric'))">a geometric key and the key after it, when both values are numbers, are non-zero and of one sign; the curves that ease a progress from 0 to 1 (motionPath, flyTo, transition and the particle curves) are not geometric.</sch:assert>
    </sch:rule>
  </sch:pattern>
</sch:schema>
