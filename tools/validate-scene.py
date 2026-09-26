#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate a scene-render document against the 1.1 contract.

Two layers:

1. XSD: schema/scene-render-1.1.xsd through xmllint (run inside the Flatpak
   SDK automatically when xmllint is not on PATH).
2. Semantics: the rules XSD 1.0 cannot express, i.e. the missing
   scene-render-1.1.sch. The rules follow the schema annotations and the
   accepted decisions in docs/schema-1.1-batch{1,2,3}-proposal.md. Every
   rule has an id (listed by --list-rules).

Element typing, attribute types and the animatable-property vocabulary are
derived from the XSD, so the tool follows schema errata without edits.

Exit status: 0 valid, 1 invalid, 2 usage or tool error.
Only the Python standard library is required; lxml is used for the XSD layer
when installed, otherwise xmllint.
"""

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
import xml.parsers.expat

XS = "{http://www.w3.org/2001/XMLSchema}"
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_SCHEMA = os.path.join(REPO, "schema", "scene-render-1.1.xsd")
V10_SCHEMA = os.path.join(REPO, "schema", "scene-v1.xsd")
SDK = "org.freedesktop.Sdk//25.08"

RULES = {
    "XSD": "document is valid against the XSD",
    "XML-DOCTYPE": "no DOCTYPE (the renderer refuses them)",
    "VERSION-GATE": "1.0 documents use only 1.0 elements and enumeration values",
    "ID-UNIQUE": "ids are unique across the document",
    "REF-RESOLVE": "every IDREF/IDREFS names an existing id",
    "REF-KIND": "every reference names an element of the expected kind",
    "PAINT-REF": "url(#id) names a paints/* element",
    "TOKEN-REF": "var(--name) names a styles/token of a fitting value",
    "TEMPLATE-REF": "{{name}} in text names a parameter",
    "PROPERTY": "animated/linked/overridden properties exist on their host",
    "KEYS": "keyframe times unique and ordered; values fit the property; curve attributes present",
    "EXPR-SYNTAX": "expressions parse in the scene-render expression language",
    "EXPR-NAME": "expressions use only built-ins, bindings and resolvable literal references",
    "EXPR-LIMIT": "expressions stay within the AST size and depth limits",
    "LINK": "link sources are well formed and resolve",
    "CYCLE": "no reference cycles (parent, matte, symbol, bus, style, prop/link)",
    "TIME": "node, clip, cue and output times are consistent with the project",
    "STRUCTURE": "cross-field rules from the schema annotations",
    "OVERRIDE": "override/bind/set targets resolve in scope and name real properties",
    "PARAM": "parameter defaults and variant values satisfy their declaration",
    "OUTPUT": "codec/container/alpha/path combinations are encodable",
    "DESTINATION": "destination URIs match their kind and carry no credentials",
    "FILES": "(--check-files) referenced local files exist and hashes match",
    "CAMERA-CUT": "the active camera does not change inside a transition window over 2.5D/3D content",
    "ISOLATION": "collapse=\"true\" is not cancelled by effects/isolate (an isolated group flattens depth)",
    "DISPLAY": "the display transform matches the colour space of the outputs",
    "SYMBOL-CLIP": "layers of a sized symbol stay inside its box (sized symbols clip)",
    "SEED-SHARED": "no fixed seed inside content that is instantiated or repeated more than once",
}

# --------------------------------------------------------------------- report


class Report:
    def __init__(self, path):
        self.path = path
        self.items = []

    def add(self, level, rule, node, attr, msg):
        assert rule in RULES, rule
        line = node.line if node is not None else 0
        where = ""
        if node is not None:
            where = "<%s%s>" % (node.tag, ' id="%s"' % node.get("id") if node.get("id") else "")
            if attr:
                where += " @" + attr
        self.items.append({"level": level, "rule": rule, "line": line,
                           "where": where, "message": msg})

    def error(self, rule, node, attr, msg):
        self.add("error", rule, node, attr, msg)

    def warn(self, rule, node, attr, msg):
        self.add("warning", rule, node, attr, msg)

    def count(self, level):
        return sum(1 for i in self.items if i["level"] == level)

# ------------------------------------------------------------ document model


class Node:
    __slots__ = ("tag", "attrib", "children", "parent", "line", "text", "type")

    def __init__(self, tag, attrib, parent, line):
        self.tag, self.attrib, self.parent, self.line = tag, attrib, parent, line
        self.children, self.text, self.type = [], "", None

    def get(self, k, d=None):
        return self.attrib.get(k, d)

    def iter(self, tag=None):
        if tag is None or self.tag == tag:
            yield self
        for c in self.children:
            yield from c.iter(tag)

    def ancestors(self):
        n = self.parent
        while n is not None:
            yield n
            n = n.parent


def parse_document(path, report):
    data = open(path, "rb").read()
    p = xml.parsers.expat.ParserCreate()
    root, stack = [None], []

    def start(tag, attrs):
        n = Node(tag, attrs, stack[-1] if stack else None, p.CurrentLineNumber)
        if stack:
            stack[-1].children.append(n)
        else:
            root[0] = n
        stack.append(n)

    def end(tag):
        stack.pop()

    def chars(s):
        if stack:
            stack[-1].text += s

    def doctype(*a):
        report.error("XML-DOCTYPE", None, None, "DOCTYPE declarations are not allowed")

    p.StartElementHandler, p.EndElementHandler = start, end
    p.CharacterDataHandler, p.StartDoctypeDeclHandler = chars, doctype
    p.Parse(data, True)
    return root[0]

# ------------------------------------------------------------- schema model

NUMERIC = {"xs:double", "xs:decimal", "xs:float", "xs:integer", "xs:int",
           "xs:long", "xs:positiveInteger", "xs:nonNegativeInteger",
           "xs:unsignedLong", "xs:unsignedInt"}


class Simple:
    """family: number|int|bool|string|enum|color|paint|length|point|numlist|
    id|idref|idrefs|uri|expr|other"""
    __slots__ = ("family", "enums", "name")

    def __init__(self, family, enums=None, name=None):
        self.family, self.enums, self.name = family, enums or set(), name


SPECIAL_SIMPLE = {
    "colorType": "color", "paintType": "paint", "paintRefType": "paint",
    "lengthType": "length", "positiveLengthType": "length",
    "pointType": "point", "numberListType": "numlist",
    "expressionString": "expr",
}


class Schema:
    def __init__(self, path):
        self.root = ET.parse(path).getroot()
        self.simple_defs = {e.get("name"): e for e in self.root if e.tag == XS + "simpleType"}
        self.complex_defs = {e.get("name"): e for e in self.root if e.tag == XS + "complexType"}
        self.agroups = {e.get("name"): e for e in self.root if e.tag == XS + "attributeGroup"}
        self.groups = {e.get("name"): e for e in self.root if e.tag == XS + "group"}
        self._simple_cache = {}
        self.types = {}          # type key -> {"attrs": {}, "children": {}}
        root_el = [e for e in self.root if e.tag == XS + "element"][0]
        self.root_name = root_el.get("name")
        self.root_type = self._complex_key(root_el)

    # simple types
    def simple(self, name, inline=None):
        if inline is not None:
            return self._simple_from(inline, None)
        if name in self._simple_cache:
            return self._simple_cache[name]
        if name in SPECIAL_SIMPLE:
            s = Simple(SPECIAL_SIMPLE[name], name=name)
        elif name == "xs:boolean":
            s = Simple("bool", name=name)
        elif name in ("xs:ID",):
            s = Simple("id", name=name)
        elif name == "xs:IDREF":
            s = Simple("idref", name=name)
        elif name == "xs:IDREFS":
            s = Simple("idrefs", name=name)
        elif name == "xs:anyURI":
            s = Simple("uri", name=name)
        elif name in NUMERIC:
            s = Simple("int" if "nteger" in name or "Long" in name or name == "xs:int" or "Int" in name else "number", name=name)
        elif name and name.startswith("xs:"):
            s = Simple("string", name=name)
        elif name in self.simple_defs:
            s = self._simple_from(self.simple_defs[name], name)
        else:
            s = Simple("other", name=name)
        self._simple_cache[name] = s
        return s

    def _simple_from(self, st, name):
        r = st.find(XS + "restriction")
        if r is not None:
            enums = {e.get("value") for e in r.findall(XS + "enumeration")}
            if enums:
                return Simple("enum", enums, name)
            base = self.simple(r.get("base"))
            return Simple(base.family, base.enums, name)
        u = st.find(XS + "union")
        if u is not None:
            fams = {self.simple(m).family for m in u.get("memberTypes").split()}
            return Simple(fams.pop() if len(fams) == 1 else "other", name=name)
        if st.find(XS + "list") is not None:
            return Simple("numlist", name=name)
        return Simple("other", name=name)

    # complex types
    def _complex_key(self, el):
        if el.get("type"):
            return el.get("type")
        key = "el:" + el.get("name")
        if key not in self.complex_defs:
            self.complex_defs[key] = el.find(XS + "complexType")
        return key

    def info(self, key):
        if key in self.types:
            return self.types[key]
        info = {"attrs": {}, "children": {}}
        self.types[key] = info
        ct = self.complex_defs.get(key)
        if ct is not None:
            self._collect(ct, info)
        return info

    def _collect(self, node, info):
        for c in node:
            t = c.tag[len(XS):]
            if t == "attribute":
                inline = c.find(XS + "simpleType")
                info["attrs"][c.get("name")] = self.simple(c.get("type"), inline)
            elif t == "attributeGroup":
                self._collect(self.agroups[c.get("ref")], info)
            elif t == "group":
                self._collect(self.groups[c.get("ref")], info)
            elif t == "element":
                info["children"][c.get("name")] = self._complex_key(c)
            elif t in ("extension", "restriction") and c.get("base"):
                base = c.get("base")
                if base in self.complex_defs:
                    b = self.info(base)
                    info["attrs"].update(b["attrs"])
                    info["children"].update(b["children"])
                self._collect(c, info)
            elif t in ("sequence", "choice", "all", "complexContent",
                       "simpleContent", "complexType"):
                self._collect(c, info)

    def assign_types(self, root):
        root.type = self.root_type
        for n in root.iter():
            ch = self.info(n.type)["children"] if n.type else {}
            for c in n.children:
                c.type = ch.get(c.tag)

    def all_types(self):
        seen, todo = set(), [self.root_type]
        while todo:
            k = todo.pop()
            if k in seen:
                continue
            seen.add(k)
            todo.extend(self.info(k)["children"].values())
        return seen

# -------------------------------------------------------- reference kinds

NODE = {"group", "sequence", "layer", "shape", "object3D", "camera",
        "particleEmitter", "instance", "include", "repeat", "adjustment"}
VISUAL_ASSET = {"image", "video", "imageSequence", "text", "vector", "lottie",
                "generator", "chart", "audiogram", "code", "formula", "generated"}
IMAGE_LIKE = {"image", "imageSequence", "video", "generator", "generated", "vector"}
AUDIO_SRC = {"audio", "video", "generated"}
TEXT_STYLE = {"textStyle"}
MARKER = {"marker", "#beat"}

# (element tag or "*", attribute) -> allowed target tags. "*" rows apply to
# every element that has the attribute. Every IDREF/IDREFS attribute in the
# schema must be covered (checked at start-up).
REF_KINDS = {
    ("key", "marker"): MARKER,
    ("poster", "marker"): MARKER,
    ("thumbnail", "marker"): MARKER,
    ("*", "startMarker"): MARKER,
    ("*", "endMarker"): MARKER,
    ("transformConstraint", "target"): NODE | {"bone", "trackData"},
    ("pattern", "asset"): IMAGE_LIKE,
    ("*", "fontAsset"): {"font"},
    ("textStyle", "basedOn"): TEXT_STYLE,
    ("span", "style"): TEXT_STYLE,
    ("text", "style"): TEXT_STYLE,
    ("chart", "textStyle"): TEXT_STYLE,
    ("cue", "style"): TEXT_STYLE,
    ("captionTrack", "style"): TEXT_STYLE,
    ("captionTrack", "activeStyle"): TEXT_STYLE,
    ("audiogram", "source"): {"audio", "audioTrack", "video", "generated"},
    ("bone", "parent"): {"bone"},
    ("*", "parent"): NODE,
    ("*", "matte"): NODE,
    ("modifier", "skeleton"): {"skeleton"},
    ("layer", "asset"): VISUAL_ASSET,
    ("*", "effects"): {"effect"},
    ("layer", "audioBus"): {"bus"},
    ("audioTrack", "bus"): {"bus"},
    ("bus", "output"): {"bus"},
    ("particleEmitter", "emitterAsset"): IMAGE_LIKE,
    ("particleEmitter", "sprite"): IMAGE_LIKE,
    ("particleEmitter", "forceFields"): {"forceField"},
    ("object3D", "material"): {"material"},
    ("object3D", "mesh"): {"mesh"},
    ("camera", "target"): NODE,
    ("camera", "focusTarget"): NODE,
    ("instance", "symbol"): {"symbol"},
    ("repeat", "over"): {"param", "data"},
    ("transition", "from"): NODE,
    ("transition", "to"): NODE,
    ("effect", "lights"): {"light"},
    ("effect", "source"): NODE | VISUAL_ASSET,
    ("constraint", "a"): NODE,
    ("constraint", "b"): NODE,
    ("trackData", "footage"): {"video", "imageSequence", "layer"},
    ("audioEffect", "sidechain"): {"audioTrack", "bus"},
    ("*", "duckUnder"): {"audioTrack", "bus"},
    ("audioTrack", "asset"): AUDIO_SRC,
    ("captionTrack", "transcribe"): {"audioTrack", "audio"},
    ("*", "safeArea"): {"safeArea"},
    ("beatGrid", "source"): {"audio", "audioTrack"},
    ("bind", "param"): {"param"},
    ("set", "param"): {"param"},
    ("bind", "target"): None,           # any element with an id
    ("colorManagement", "looks"): {"look"},
    ("accessibility", "audioDescription"): {"audioTrack"},
    ("scene360", "viewportCamera"): {"camera"},
    ("output", "layout"): {"layout"},
    ("output", "variant"): {"variant"},
    ("output", "captions"): {"captionTrack"},
    ("output", "burnCaptions"): {"captionTrack"},
}


def ref_kinds(tag, attr):
    if (tag, attr) in REF_KINDS:
        return True, REF_KINDS[(tag, attr)]
    if ("*", attr) in REF_KINDS:
        return True, REF_KINDS[("*", attr)]
    return False, None

# ------------------------------------------------------------ value syntax

NUM_RE = re.compile(r"^[-+]?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?$")
REL_LEN_RE = re.compile(r"^-?(\d+(\.\d*)?|\.\d+)(%|vw|vh|vmin|vmax)$")
COLOR_RE = re.compile(
    r"^(#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?|(0(\.\d+)?|1(\.0+)?|\.\d+)(,(0(\.\d+)?|1(\.0+)?|\.\d+)){2,3}|var\(--[A-Za-z0-9_\-]+\))$")
PAINT_REF_RE = re.compile(r"^url\(#([A-Za-z_][A-Za-z0-9_.\-]*)\)$")
POINT_RE = re.compile(r"^\s*[-+]?[\d.eE+-]+\s*,\s*[-+]?[\d.eE+-]+\s*$")
VAR_RE = re.compile(r"var\(--([A-Za-z0-9_\-]+)\)")
TEMPLATE_RE = re.compile(r"\{\{\s*([^}\s]+)\s*\}\}")
PATH_RE = re.compile(r"^[\sMmLlHhVvCcSsQqTtAaZz0-9eE.,+-]*$")
PATH_CMD_RE = re.compile(r"[MmLlHhVvCcSsQqTtAaZz]")


def is_num(s):
    return bool(NUM_RE.match(s.strip()))


def fnum(s, default=None):
    try:
        v = float(s)
        return v if math.isfinite(v) else default
    except (TypeError, ValueError):
        return default


def value_fits(family, value, attr=None):
    v = value.strip()
    if family in ("number", "int"):
        return is_num(v)
    if family == "length":
        return is_num(v) or bool(REL_LEN_RE.match(v))
    if family == "color":
        return bool(COLOR_RE.match(v))
    if family == "paint":
        return bool(COLOR_RE.match(v) or PAINT_REF_RE.match(v))
    if family == "point":
        return bool(POINT_RE.match(v))
    if family == "path":
        return bool(PATH_RE.match(v)) and bool(PATH_CMD_RE.search(v))
    return True

# ------------------------------------------------------ expression language

BUILTIN_VARS = {"time", "frame", "value", "index", "count", "seed",
                "textIndex", "textTotal", "true", "false", "PI", "E"}
MATH_FUNCS = {"abs": (1, 1), "floor": (1, 1), "ceil": (1, 1), "round": (1, 1),
              "trunc": (1, 1), "sign": (1, 1), "sqrt": (1, 1), "cbrt": (1, 1),
              "exp": (1, 1), "log": (1, 1), "log2": (1, 1), "log10": (1, 1),
              "pow": (2, 2), "sin": (1, 1), "cos": (1, 1), "tan": (1, 1),
              "asin": (1, 1), "acos": (1, 1), "atan": (1, 1), "atan2": (2, 2),
              "hypot": (1, 8), "min": (1, 64), "max": (1, 64)}
FUNCS = {"param": (1, 1), "prop": (1, 2), "valueAtTime": (1, 1),
         "wiggle": (2, 4), "noise": (1, 3), "random": (0, 2),
         "loopIn": (0, 2), "loopOut": (0, 2), "linear": (3, 5), "ease": (3, 5),
         "easeIn": (3, 5), "easeOut": (3, 5), "clamp": (3, 3), "lerp": (3, 3),
         "smoothstep": (3, 3), "spring": (4, 4), "audioAmplitude": (1, 2),
         "beat": (0, 0), "markerTime": (1, 1)}
FUNCS.update(MATH_FUNCS)
LITERAL_ARG_FUNCS = {"param", "prop", "markerTime", "audioAmplitude", "loopIn", "loopOut"}
MAX_AST_NODES, MAX_DEPTH = 4096, 64

TOKEN_RE = re.compile(r"""
    (?P<ws>\s+)|(?P<num>(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?)|
    (?P<str>"([^"\\]|\\.)*"|'([^'\\]|\\.)*')|(?P<id>[A-Za-z_$][A-Za-z0-9_$]*)|
    (?P<op>\*\*|===|!==|==|!=|<=|>=|&&|\|\||[-+*/%<>!?:()\[\],.;=])""", re.X)


class ExprError(Exception):
    pass


class Expr:
    """Recursive-descent parser for the B2-1 grammar:
    program := ('const' id '=' expr ';')* expr ';'?
    expr    := cond; cond := or ('?' expr ':' expr)?; or := and ('||' and)*
    and := eq ('&&' eq)*; eq := rel (('=='|'!='|'==='|'!==') rel)*
    rel := add (('<'|'<='|'>'|'>=') add)*; add := mul (('+'|'-') mul)*
    mul := pow (('*'|'/'|'%') pow)*; pow := unary ('**' pow)?
    unary := ('-'|'+'|'!') unary | postfix
    postfix := primary ('(' args ')' | '[' expr ']' | '.' id)*
    primary := number | string | id | '(' expr ')' | '[' elements ']'"""

    def __init__(self, src):
        self.toks, pos = [], 0
        while pos < len(src):
            m = TOKEN_RE.match(src, pos)
            if not m:
                raise ExprError("unexpected character %r at offset %d" % (src[pos], pos))
            pos = m.end()
            kind = m.lastgroup
            if kind != "ws":
                self.toks.append((kind, m.group(kind), m.start()))
        self.toks.append(("end", "", len(src)))
        self.i, self.nodes, self.depth, self.max_depth = 0, 0, 0, 0
        self.calls, self.names, self.members, self.bindings = [], [], [], set()

    def peek(self, v=None):
        k, t, _ = self.toks[self.i]
        return t == v and k in ("op", "id") if v is not None else (k, t)

    def take(self, v=None):
        k, t, off = self.toks[self.i]
        if v is not None and (t != v or k not in ("op", "id")):
            raise ExprError("expected %r at offset %d, found %r" % (v, off, t or "end"))
        self.i += 1
        return k, t, off

    def node(self, n):
        self.nodes += 1
        return n

    def parse(self):
        while self.peek("const"):
            self.take("const")
            k, name, off = self.take()
            if k != "id":
                raise ExprError("binding name expected at offset %d" % off)
            if name in BUILTIN_VARS or name in FUNCS or name == "Math":
                raise ExprError("binding %r shadows a built-in" % name)
            self.take("=")
            self.expr()
            self.take(";")
            self.bindings.add(name)
        tree = self.expr()
        if self.peek(";"):
            self.take(";")
        if self.toks[self.i][0] != "end":
            raise ExprError("unexpected %r at offset %d" % (self.toks[self.i][1], self.toks[self.i][2]))
        return tree

    def expr(self):
        self.depth += 1
        self.max_depth = max(self.max_depth, self.depth)
        if self.depth > MAX_DEPTH:
            raise ExprError("nesting deeper than %d" % MAX_DEPTH)
        n = self.binary(0)
        if self.peek("?"):
            self.take("?")
            a = self.expr()
            self.take(":")
            b = self.expr()
            n = self.node(("?", n, a, b))
        self.depth -= 1
        return n

    LEVELS = [("||",), ("&&",), ("==", "!=", "===", "!=="),
              ("<", "<=", ">", ">="), ("+", "-"), ("*", "/", "%")]

    def binary(self, level):
        if level == len(self.LEVELS):
            return self.power()
        n = self.binary(level + 1)
        while self.toks[self.i][0] == "op" and self.toks[self.i][1] in self.LEVELS[level]:
            op = self.take()[1]
            n = self.node((op, n, self.binary(level + 1)))
        return n

    def power(self):
        n = self.unary()
        if self.peek("**"):
            self.take("**")
            n = self.node(("**", n, self.power()))
        return n

    def unary(self):
        if self.toks[self.i][0] == "op" and self.toks[self.i][1] in ("-", "+", "!"):
            op = self.take()[1]
            return self.node(("u" + op, self.unary()))
        return self.postfix()

    def postfix(self):
        n = self.primary()
        while True:
            if self.peek("("):
                if n[0] not in ("id", "member"):
                    raise ExprError("only named built-ins can be called")
                self.take("(")
                args = []
                if not self.peek(")"):
                    args.append(self.expr())
                    while self.peek(","):
                        self.take(",")
                        args.append(self.expr())
                self.take(")")
                n = self.node(("call", n, args))
                self.calls.append(n)
            elif self.peek("["):
                self.take("[")
                n = self.node(("index", n, self.expr()))
                self.take("]")
            elif self.peek("."):
                self.take(".")
                k, name, off = self.take()
                if k != "id":
                    raise ExprError("member name expected at offset %d" % off)
                n = self.node(("member", n, name))
                self.members.append(n)
            else:
                return n

    def primary(self):
        k, t, off = self.take()
        if k == "num":
            return self.node(("num", float(t)))
        if k == "str":
            return self.node(("str", bytes(t[1:-1], "utf-8").decode("unicode_escape")))
        if k == "id":
            n = self.node(("id", t, off))
            self.names.append(n)
            return n
        if t == "(":
            n = self.expr()
            self.take(")")
            return n
        if t == "[":
            items = []
            if not self.peek("]"):
                items.append(self.expr())
                while self.peek(","):
                    self.take(",")
                    items.append(self.expr())
            self.take("]")
            return self.node(("array", items))
        raise ExprError("unexpected %r at offset %d" % (t or "end", off))

# ------------------------------------------------------------------ checker

ALIASES = {"position.x": "x", "position.y": "y", "position.z": "z",
           "scale.x": "scaleX", "scale.y": "scaleY", "scale.z": "scaleZ",
           "anchor.x": "anchorX", "anchor.y": "anchorY",
           "rotation.x": "rotationX", "rotation.y": "rotationY",
           "skew.x": "skewX", "skew.y": "skewY", "depth": "zDepth"}
ANIMATABLE = {"number", "int", "length", "color", "paint", "point"}
PATH_ATTRS = {"path", "emitterPath"}
EXTRA_PROPS = {"layer": {"source.time": "number"},
               "motionPath": {"progress": "number"},
               "transition": {"progress": "number"},
               "textAnimator": {"selector": "number"}}

CONTAINERS = {"h264": {"mp4", "mov", "mkv"}, "h265": {"mp4", "mov", "mkv"},
              "ffv1": {"mkv", "mov"}, "av1": {"mp4", "mkv", "webm"},
              "vp9": {"webm", "mkv", "mp4"}, "prores": {"mov", "mxf"},
              "dnxhr": {"mov", "mxf"}, "gif": set(), "apng": set(),
              "webp": set(), "png-sequence": set(), "jpeg-sequence": set(),
              "exr-sequence": set(), "tiff-sequence": set(),
              "audio-only": {"wav", "m4a", "mp3", "mkv"}}
ALPHA_CODECS = {"prores", "vp9", "ffv1", "png-sequence", "tiff-sequence",
                "exr-sequence", "webp", "apng", "gif"}
SEQ_PATTERN = re.compile(r"%0?\d*d|#+")
DEST_SCHEMES = {"file": ("file", ""), "s3": ("s3", "https"), "gcs": ("gs", "https"),
                "azure-blob": ("https",), "http-put": ("http", "https"),
                "webhook": ("http", "https"), "sftp": ("sftp",)}
QR_CAPACITY = {"L": 2953, "M": 2331, "Q": 1663, "H": 1273}
URI_ATTRS_HASH = {"src": "sha256", "cache": "cacheSha256"}


class Checker:
    def __init__(self, doc, schema, report, path, check_files):
        self.doc, self.s, self.r = doc, schema, report
        self.path, self.dir, self.check_files = path, os.path.dirname(os.path.abspath(path)), check_files
        self.ids = {}
        self.project = next((c for c in doc.children if c.tag == "project"), None)
        self.duration = fnum(self.project.get("duration"), 0.0) if self.project is not None else 0.0
        self.fps = self._fps(self.project.get("fps") if self.project is not None else None)
        self.version = doc.get("version")
        self.prop_edges = {}

    @staticmethod
    def _fps(s):
        if not s:
            return None
        a, _, b = s.partition("/")
        try:
            return int(a) / int(b or 1)
        except (ValueError, ZeroDivisionError):
            return None

    def attr_simple(self, node, attr):
        if node.type is None:
            return None
        return self.s.info(node.type)["attrs"].get(attr)

    # ----------------------------------------------------------- ids/refs
    def collect_ids(self):
        for n in self.doc.iter():
            for a, v in n.attrib.items():
                s = self.attr_simple(n, a)
                if s is not None and s.family == "id":
                    if v in self.ids:
                        self.r.error("ID-UNIQUE", n, a, "id %r already used at line %d" % (v, self.ids[v].line))
                    else:
                        self.ids[v] = n
        # generated beat/bar ids (B1-5): beat.N and bar.N over the project
        for g in self.doc.iter("beatGrid"):
            bpm, off = fnum(g.get("bpm"), 0), fnum(g.get("offset", "0"), 0)
            per_bar = int(fnum(g.get("beatsPerBar", "4"), 4))
            if bpm and bpm > 0:
                beats = int(max(0.0, self.duration - off) * bpm / 60.0) + 1
                for i in range(beats + 1):
                    self.ids.setdefault("beat.%d" % i, Node("#beat", {}, None, g.line))
                for i in range(beats // max(per_bar, 1) + 2):
                    self.ids.setdefault("bar.%d" % i, Node("#beat", {}, None, g.line))

    def target_ok(self, allowed, target):
        return allowed is None or target.tag in allowed

    def check_refs(self):
        for n in self.doc.iter():
            for a, v in n.attrib.items():
                s = self.attr_simple(n, a)
                if s is None or s.family not in ("idref", "idrefs"):
                    continue
                known, allowed = ref_kinds(n.tag, a)
                for ref in (v.split() if s.family == "idrefs" else [v]):
                    t = self.ids.get(ref)
                    if t is None:
                        self.r.error("REF-RESOLVE", n, a, "no element has id %r" % ref)
                    elif not self.target_ok(allowed, t):
                        self.r.error("REF-KIND", n, a, "%r is a <%s>, expected one of %s"
                                     % (ref, t.tag, ", ".join("<%s>" % k for k in sorted(allowed) if not k.startswith("#"))))
                    else:
                        self.ref_detail(n, a, t)

    def ref_detail(self, n, a, t):
        tag = n.tag
        if tag == "layer" and a == "asset" and t.tag == "generated" and t.get("kind") not in ("image", "video"):
            self.r.error("REF-KIND", n, a, "generated asset %r has kind %r; layers need image or video" % (t.get("id"), t.get("kind")))
        if tag == "audioTrack" and a == "asset":
            if t.tag == "generated" and t.get("kind") in ("image", "video") and t.get("kind") != "video":
                self.r.error("REF-KIND", n, a, "generated asset %r is not audio" % t.get("id"))
            if t.tag == "video" and t.get("hasAudio", "false") != "true":
                self.r.warn("REF-KIND", n, a, "video %r does not declare hasAudio=\"true\"" % t.get("id"))
        if tag == "constraint" and a in ("a", "b") and not any(c.tag in ("rigidBody", "softBody") for c in t.children):
            self.r.error("REF-KIND", n, a, "%r has no rigidBody or softBody" % t.get("id"))
        if tag == "repeat" and a == "over" and t.tag == "param" and t.get("type") != "list":
            self.r.error("REF-KIND", n, a, "param %r must have type=\"list\"" % t.get("id"))
        if tag == "transformConstraint" and a == "target":
            if (n.get("type") == "track") != (t.tag == "trackData"):
                self.r.error("REF-KIND", n, a, "type=%r cannot target a <%s>" % (n.get("type"), t.tag))
        if tag == "output" and a == "burnCaptions" and t.get("mode", "burn") == "sidecar":
            self.r.warn("REF-KIND", n, a, "caption track %r has mode=\"sidecar\"" % t.get("id"))
        if tag == "trackData" and a == "footage" and t.tag == "layer":
            asset = self.ids.get(t.get("asset", ""))
            if asset is not None and asset.tag not in ("video", "imageSequence"):
                self.r.error("REF-KIND", n, a, "layer %r does not show video footage" % t.get("id"))

    # ---------------------------------------------------- paints/tokens
    def check_values(self):
        tokens = {}
        for t in self.doc.iter("token"):
            if t.get("name") in tokens:
                self.r.error("ID-UNIQUE", t, "name", "token %r defined twice" % t.get("name"))
            tokens[t.get("name")] = t
        params = {p.get("id") for p in self.doc.iter("param") if p.parent is not None and p.parent.tag == "parameters"}
        for n in self.doc.iter():
            for a, v in n.attrib.items():
                s = self.attr_simple(n, a)
                for name in VAR_RE.findall(v):
                    tok = tokens.get(name)
                    if tok is None:
                        self.r.error("TOKEN-REF", n, a, "var(--%s) has no styles/token" % name)
                    elif s is not None and s.family in ("color", "paint") and not COLOR_RE.match(tok.get("value", "").strip()):
                        self.r.error("TOKEN-REF", n, a, "token --%s = %r is not a colour" % (name, tok.get("value")))
                    elif s is not None and s.family in ("color", "paint") and VAR_RE.search(tok.get("value", "")):
                        self.r.error("TOKEN-REF", n, a, "token --%s refers to another token" % name)
                m = PAINT_REF_RE.match(v.strip())
                if m and s is not None and s.family == "paint":
                    t = self.ids.get(m.group(1))
                    if t is None:
                        self.r.error("PAINT-REF", n, a, "url(#%s) does not resolve" % m.group(1))
                    elif t.parent is None or t.parent.tag != "paints":
                        self.r.error("PAINT-REF", n, a, "url(#%s) is a <%s>, not a paint" % (m.group(1), t.tag))
        for n in list(self.doc.iter("text")) + list(self.doc.iter("span")):
            src = (n.get("text") or "") + (n.text if n.tag == "span" else "")
            for name in TEMPLATE_RE.findall(src):
                if name not in params:
                    self.r.error("TEMPLATE-REF", n, "text", "{{%s}} names no parameter" % name)

    # -------------------------------------------------------- properties
    def host_props(self, host):
        props = {}
        if host is None or host.type is None:
            return props
        for a, s in self.s.info(host.type)["attrs"].items():
            if s.family in ANIMATABLE:
                props[a] = s.family
            elif a in PATH_ATTRS or (a == "path" and s.family == "string"):
                props[a] = "path"
            elif a in ("lift", "gamma", "gain", "slope", "offset", "power", "corners", "cornerRadii", "morphWeights"):
                props[a] = "string"
        props.update(EXTRA_PROPS.get(host.tag, {}))
        return props

    def resolve_prop(self, host, prop):
        props = self.host_props(host)
        name = ALIASES.get(prop, prop)
        if name in props:
            return props[name]
        return None

    def animation_host(self, anim):
        host = anim.parent
        return host

    def check_animation(self):
        for anim in self.doc.iter():
            if anim.tag not in ("animate", "expression", "link"):
                continue
            host = self.animation_host(anim)
            prop = anim.get("property", "")
            if host is not None and host.tag == "motionPath" and prop != "progress":
                self.r.error("PROPERTY", anim, "property", "motionPath animates only \"progress\"")
                continue
            fam = self.resolve_prop(host, prop)
            if fam is None:
                self.r.error("PROPERTY", anim, "property", "<%s> has no animatable property %r" % (host.tag if host else "?", prop))
                continue
            if anim.tag == "animate":
                self.check_keys(anim, fam)
            elif anim.tag == "expression":
                self.check_expression(anim, anim.text or "", host, prop)
            else:
                self.check_link(anim, host, prop)
        for n in self.doc.iter("timeRemap"):
            self.check_keys(n, "number")

    def check_keys(self, anim, fam):
        keys = [k for k in anim.children if k.tag == "key"]
        times = []
        for i, k in enumerate(keys):
            t = fnum(k.get("time"))
            if k.get("marker"):
                mk = self.ids.get(k.get("marker"))
                if mk is not None and mk.tag == "marker":
                    t = (fnum(mk.get("time"), 0) or 0) + (t or 0)
                elif mk is not None and mk.tag == "#beat":
                    t = self.beat_time(k.get("marker")) + (t or 0)
            times.append(t)
            v = k.get("value", "")
            if fam != "string" and not value_fits(fam, v):
                self.r.error("KEYS", k, "value", "%r is not a valid %s value" % (v, fam))
            interp = k.get("interpolation") or anim.get("defaultInterpolation", "linear")
            last = i == len(keys) - 1
            if interp == "steps" and not k.get("steps") and not last:
                self.r.error("KEYS", k, "steps", "interpolation=\"steps\" needs @steps")
            if interp == "cubic-bezier" and not last:
                if k.get("bezier"):
                    parts = [fnum(x) for x in re.split(r"[\s,]+", k.get("bezier").strip())]
                    if len(parts) != 4 or None in parts or not (0 <= parts[0] <= 1 and 0 <= parts[2] <= 1):
                        self.r.error("KEYS", k, "bezier", "bezier must be \"x1,y1,x2,y2\" with x1, x2 in [0,1]")
                elif not (k.get("easeOut") or keys[i + 1].get("easeIn")):
                    self.r.error("KEYS", k, "interpolation", "cubic-bezier needs @bezier, or @easeOut here / @easeIn on the next key")
        if fam == "path" and len(keys) > 1:
            shapes = {tuple(PATH_CMD_RE.findall(k.get("value", "")).__iter__()) for k in keys}
            if len({tuple(c.upper() for c in s) for s in shapes}) > 1:
                self.r.error("KEYS", anim, "property", "path keys must share the same command structure to morph")
        seen = {}
        for k, t in zip(keys, times):
            if t is None:
                continue
            if t in seen:
                self.r.error("KEYS", k, "time", "key time %g repeats line %d" % (t, seen[t].line))
            seen[t] = k
        clean = [t for t in times if t is not None]
        if clean != sorted(clean):
            self.r.warn("KEYS", anim, None, "keys are not in time order")

    # -------------------------------------------------------- expressions
    def in_text_animator(self, n):
        return any(a.tag == "textAnimator" for a in n.ancestors())

    def repeat_var(self, n):
        for a in n.ancestors():
            if a.tag == "repeat":
                return a.get("var", "item") if a.get("over") else None
        return None

    def check_expression(self, node, src, host, prop, attr=None):
        where = attr
        if len(src.encode()) > 65536:
            self.r.error("EXPR-LIMIT", node, where, "expression longer than 64 KiB")
            return
        try:
            e = Expr(src)
            e.parse()
        except ExprError as ex:
            self.r.error("EXPR-SYNTAX", node, where, "%s in %r" % (ex, src.strip()[:80]))
            return
        except RecursionError:
            self.r.error("EXPR-LIMIT", node, where, "expression nests too deeply")
            return
        if e.nodes > MAX_AST_NODES:
            self.r.error("EXPR-LIMIT", node, where, "%d AST nodes (limit %d)" % (e.nodes, MAX_AST_NODES))
        rvar = self.repeat_var(node)
        callee_ids = set()
        for c in e.calls:
            f = c[1]
            if f[0] == "member":
                callee_ids.add(id(f))
                if f[1][0] != "id" or f[1][1] != "Math" or f[2] not in MATH_FUNCS:
                    self.r.error("EXPR-NAME", node, where, "unknown function %s" % self.render_callee(f))
                    continue
                name = f[2]
            else:
                callee_ids.add(id(f))
                name = f[1]
                if name not in FUNCS:
                    self.r.error("EXPR-NAME", node, where, "unknown function %s()" % name)
                    continue
            lo, hi = FUNCS[name]
            if not lo <= len(c[2]) <= hi:
                self.r.error("EXPR-NAME", node, where, "%s() takes %d..%d arguments, got %d" % (name, lo, hi, len(c[2])))
                continue
            self.check_call(node, where, name, c[2], host, prop)
        for m in e.members:
            base = m[1]
            if base[0] == "id" and base[1] == "Math":
                if m[2] not in MATH_FUNCS and m[2] not in ("PI", "E") and id(m) not in callee_ids:
                    self.r.error("EXPR-NAME", node, where, "unknown Math.%s" % m[2])
                elif m[2] in MATH_FUNCS and id(m) not in callee_ids:
                    self.r.error("EXPR-NAME", node, where, "Math.%s must be called" % m[2])
            elif base[0] == "id" and rvar and base[1] == rvar:
                pass
            elif base[0] == "call" and base[1][0] == "id" and base[1][1] == "param":
                pass
            else:
                self.r.error("EXPR-NAME", node, where, "member access .%s is only allowed on Math, the repeat item and param()" % m[2])
        for n in e.names:
            name = n[1]
            if name in BUILTIN_VARS or name in e.bindings or name == "Math" or (rvar and name == rvar):
                if name in ("textIndex", "textTotal") and not self.in_text_animator(node):
                    self.r.error("EXPR-NAME", node, where, "%s is only defined inside textAnimator" % name)
                continue
            if any(id(n) == id(c[1]) for c in e.calls):
                continue                                     # callee, checked above
            self.r.error("EXPR-NAME", node, where, "unknown name %r" % name)

    @staticmethod
    def render_callee(f):
        return "%s.%s()" % (f[1][1] if f[1][0] == "id" else "?", f[2])

    def lit(self, node, where, fname, arg):
        if arg[0] != "str":
            self.r.error("EXPR-NAME", node, where, "%s() needs a string literal argument (static dependency graph)" % fname)
            return None
        return arg[1]

    def check_call(self, node, where, name, args, host, prop):
        if name not in LITERAL_ARG_FUNCS:
            return
        if name in ("loopIn", "loopOut"):
            if args:
                t = self.lit(node, where, name, args[0])
                if t is not None and t not in ("cycle", "pingpong", "offset", "continue"):
                    self.r.error("EXPR-NAME", node, where, "%s type %r is not cycle|pingpong|offset|continue" % (name, t))
            return
        a0 = self.lit(node, where, name, args[0])
        if a0 is None:
            return
        if name == "param":
            p = self.ids.get(a0)
            if p is None or p.tag != "param":
                self.r.error("EXPR-NAME", node, where, "param(%r) names no parameter" % a0)
        elif name == "markerTime":
            m = self.ids.get(a0)
            if m is None or m.tag not in ("marker", "#beat"):
                self.r.error("EXPR-NAME", node, where, "markerTime(%r) names no marker" % a0)
        elif name == "audioAmplitude":
            t = self.ids.get(a0)
            if t is None or t.tag not in ("audioTrack", "bus"):
                self.r.error("EXPR-NAME", node, where, "audioAmplitude(%r) names no audio track or bus" % a0)
            if len(args) > 1:
                b = self.lit(node, where, name, args[1])
                if b is not None and b not in ("low", "mid", "high"):
                    self.r.error("EXPR-NAME", node, where, "band %r is not low|mid|high" % b)
        elif name == "prop":
            if len(args) == 2:
                p = self.lit(node, where, name, args[1])
                if p is None:
                    return
                self.check_prop_ref(node, where, a0, p, host, prop)
            else:
                cands = []
                for i in range(len(a0) - 1, 0, -1):
                    if a0[i] == ".":
                        tid, tp = a0[:i], a0[i + 1:]
                        if tid in self.ids and self.resolve_prop(self.ids[tid], tp):
                            cands.append((tid, tp))
                if not cands:
                    self.r.error("EXPR-NAME", node, where, "prop(%r) does not name id.property" % a0)
                elif len(cands) > 1:
                    self.r.error("EXPR-NAME", node, where, "prop(%r) is ambiguous (%s); use prop(id, property)"
                                 % (a0, ", ".join("%s + %s" % c for c in cands)))
                else:
                    self.check_prop_ref(node, where, cands[0][0], cands[0][1], host, prop)

    def check_prop_ref(self, node, where, tid, tprop, host, prop):
        t = self.ids.get(tid)
        if t is None:
            self.r.error("EXPR-NAME", node, where, "prop(): no element has id %r" % tid)
            return
        if not self.resolve_prop(t, tprop):
            self.r.error("EXPR-NAME", node, where, "prop(): <%s id=%r> has no property %r" % (t.tag, tid, tprop))
            return
        if host is not None and host.get("id"):
            src = (host.get("id"), ALIASES.get(prop, prop))
            dst = (tid, ALIASES.get(tprop, tprop))
            self.prop_edges.setdefault(src, set()).add(dst)

    def check_link(self, link, host, prop):
        src = link.get("source", "")
        if src.startswith("param:"):
            p = self.ids.get(src[6:])
            if p is None or p.tag != "param":
                self.r.error("LINK", link, "source", "%r names no parameter" % src)
        elif src.startswith("audio:"):
            parts = src[6:].split(":")
            t = self.ids.get(parts[0])
            if t is None or t.tag not in ("audioTrack", "bus"):
                self.r.error("LINK", link, "source", "%r names no audio track" % src)
            if len(parts) > 2 or (len(parts) == 2 and parts[1] not in ("low", "mid", "high")):
                self.r.error("LINK", link, "source", "band must be low|mid|high")
        elif src.startswith("marker:"):
            m = self.ids.get(src[7:])
            if m is None or m.tag not in ("marker", "#beat"):
                self.r.error("LINK", link, "source", "%r names no marker" % src)
        else:
            cands = [(src[:i], src[i + 1:]) for i in range(len(src) - 1, 0, -1)
                     if src[i] == "." and src[:i] in self.ids and self.resolve_prop(self.ids[src[:i]], src[i + 1:])]
            if not cands:
                self.r.error("LINK", link, "source", "%r is not nodeId.property, param:, audio: or marker:" % src)
            elif len(cands) > 1:
                self.r.error("LINK", link, "source", "%r is ambiguous" % src)
            elif host is not None and host.get("id"):
                self.prop_edges.setdefault((host.get("id"), ALIASES.get(prop, prop)), set()).add(
                    (cands[0][0], ALIASES.get(cands[0][1], cands[0][1])))
        lo, hi = fnum(link.get("min")), fnum(link.get("max"))
        if lo is not None and hi is not None and lo > hi:
            self.r.error("LINK", link, "min", "min > max")

    def check_conditions(self):
        for n in self.doc.iter():
            if "condition" in n.attrib:
                self.check_expression(n, n.get("condition"), None, None, "condition")

    # --------------------------------------------------------------- cycles
    def find_cycle(self, edges, label, node_of):
        state = {}
        for start in list(edges):
            stack = [(start, iter(edges.get(start, ())))]
            path = [start]
            state[start] = 1
            while stack:
                cur, it = stack[-1]
                nxt = next(it, None)
                if nxt is None:
                    state[cur] = 2
                    stack.pop()
                    path.pop()
                    continue
                st = state.get(nxt, 0)
                if st == 1:
                    cyc = path[path.index(nxt):] + [nxt]
                    self.r.error("CYCLE", node_of(cyc[0]), None, "%s cycle: %s" % (label, " -> ".join(map(str, cyc))))
                    return
                if st == 0:
                    state[nxt] = 1
                    path.append(nxt)
                    stack.append((nxt, iter(edges.get(nxt, ()))))

    def check_cycles(self):
        def by_id(i):
            return self.ids.get(i if isinstance(i, str) else i[0])
        parent, matte, bus, style, sym = {}, {}, {}, {}, {}
        for n in self.doc.iter():
            i = n.get("id")
            if n.get("parent") and n.tag != "bone":
                parent.setdefault(i, set()).add(n.get("parent"))
            for c in n.children:
                if c.tag == "transformConstraint" and c.get("type") == "parent" and c.get("target"):
                    parent.setdefault(i, set()).add(c.get("target"))
            if n.tag == "bone" and n.get("parent"):
                parent.setdefault(i, set()).add(n.get("parent"))
            if n.get("matte") and n.tag != "transition":
                if n.get("matte") == i:
                    self.r.error("CYCLE", n, "matte", "a node cannot be its own matte")
                matte.setdefault(i, set()).add(n.get("matte"))
            if n.tag == "bus" and n.get("output"):
                bus.setdefault(i, set()).add(n.get("output"))
            if n.tag == "textStyle" and n.get("basedOn"):
                style.setdefault(i, set()).add(n.get("basedOn"))
            if n.tag == "symbol":
                sym[i] = {x.get("symbol") for x in n.iter("instance") if x.get("symbol")}
        for edges, label in ((parent, "parent"), (matte, "matte"), (bus, "bus output"),
                             (style, "textStyle basedOn"), (sym, "symbol instance")):
            self.find_cycle(edges, label, by_id)
        self.find_cycle(self.prop_edges, "prop()/link dependency", by_id)
        for n in self.doc.iter():
            if n.tag in ("audioTrack", "bus") and n.get("id") in (n.get("duckUnder") or "").split():
                self.r.error("CYCLE", n, "duckUnder", "a track cannot duck under itself")

    # ----------------------------------------------------------- overrides
    def symbol_scope(self, inst):
        sym = self.ids.get(inst.get("symbol", ""))
        return sym if sym is not None and sym.tag == "symbol" else None

    def resolve_scoped(self, scope_root, path):
        parts, root = path.split("/"), scope_root
        for i, p in enumerate(parts):
            hit = next((n for n in root.iter() if n.get("id") == p and n is not root), None)
            if hit is None:
                return None
            if i < len(parts) - 1:
                if hit.tag != "instance":
                    return None
                root = self.symbol_scope(hit)
                if root is None:
                    return None
            else:
                return hit
        return None

    def target_attr_ok(self, t, prop):
        if t.type is not None and prop in self.s.info(t.type)["attrs"]:
            return True
        if self.resolve_prop(t, prop):
            return True
        if t.tag == "layer" and prop == "text":            # B2 S8, adopted
            asset = self.ids.get(t.get("asset", ""))
            return asset is not None and asset.tag == "text"
        return False

    def check_overrides(self):
        for o in self.doc.iter("override"):
            owner = o.parent
            if owner.tag == "instance":
                scope = self.symbol_scope(owner)
                t = self.resolve_scoped(scope, o.get("target", "")) if scope is not None else None
                if scope is not None and t is None:
                    self.r.error("OVERRIDE", o, "target", "%r is not inside symbol %r" % (o.get("target"), owner.get("symbol")))
            elif owner.tag == "include":
                t = None                                     # resolved against the included file
            else:
                t = self.ids.get(o.get("target", ""))
                if t is None:
                    self.r.error("OVERRIDE", o, "target", "no element has id %r" % o.get("target"))
            if t is not None:
                prop = o.get("property", "")
                if not self.target_attr_ok(t, prop):
                    self.r.error("OVERRIDE", o, "property", "<%s> has no property %r" % (t.tag, prop))
                else:
                    s = self.attr_simple(t, ALIASES.get(prop, prop))
                    if s is not None and s.family in ("idref", "idrefs"):
                        known, allowed = ref_kinds(t.tag, prop)
                        for ref in o.get("value", "").split():
                            hit = self.ids.get(ref)
                            if hit is None:
                                self.r.error("OVERRIDE", o, "value", "no element has id %r" % ref)
                            elif not self.target_ok(allowed, hit):
                                self.r.error("OVERRIDE", o, "value", "%r is a <%s>, not a valid %s for <%s>" % (ref, hit.tag, prop, t.tag))
                    elif s is not None and s.family == "enum" and o.get("value") not in s.enums:
                        self.r.error("OVERRIDE", o, "value", "%r is not one of %s" % (o.get("value"), sorted(s.enums)))
                    elif s is not None and s.family in ANIMATABLE and not value_fits(s.family, o.get("value", "")):
                        self.r.error("OVERRIDE", o, "value", "%r is not a valid %s" % (o.get("value"), s.family))
        for b in self.doc.iter("bind"):
            t = self.ids.get(b.get("target", ""))
            if t is not None and not self.target_attr_ok(t, b.get("property", "")):
                self.r.error("OVERRIDE", b, "property", "<%s> has no property %r" % (t.tag, b.get("property")))
            m = b.get("map")
            if m and not all("=" in p for p in m.split(";") if p):
                self.r.error("OVERRIDE", b, "map", "map must be \"in=out;in=out\"")

    # --------------------------------------------------------------- params
    def param_value_ok(self, p, v):
        t = p.get("type")
        if t == "number" or t == "time":
            x = fnum(v)
            if x is None:
                return "not a number"
            lo, hi = fnum(p.get("min")), fnum(p.get("max"))
            if lo is not None and x < lo or hi is not None and x > hi:
                return "outside [%s, %s]" % (p.get("min", "-inf"), p.get("max", "inf"))
        elif t == "boolean" and v not in ("true", "false", "1", "0"):
            return "not a boolean"
        elif t == "color" and not COLOR_RE.match(v):
            return "not a colour"
        elif t == "enum":
            opts = [o.strip() for o in re.split(r"[|,]", p.get("options", "")) if o.strip()]
            if v not in opts:
                return "not one of %s" % opts
        elif t == "asset":
            a = self.ids.get(v)
            if a is None or a.parent is None or a.parent.tag != "assets":
                return "names no asset"
        elif t == "string":
            if p.get("maxLength") and len(v) > int(p.get("maxLength")):
                return "longer than maxLength"
            if p.get("pattern"):
                try:
                    if not re.fullmatch(p.get("pattern"), v):
                        return "does not match pattern"
                except re.error:
                    return None
        return None

    def check_params(self):
        for p in self.doc.iter("param"):
            if p.parent is None or p.parent.tag != "parameters":
                continue
            if p.get("type") == "enum" and not p.get("options"):
                self.r.error("PARAM", p, "options", "enum parameter needs @options")
            if p.get("pattern"):
                try:
                    re.compile(p.get("pattern"))
                except re.error as ex:
                    self.r.error("PARAM", p, "pattern", "invalid pattern: %s" % ex)
            if p.get("default") is not None:
                why = self.param_value_ok(p, p.get("default"))
                if why:
                    self.r.error("PARAM", p, "default", "%r is %s" % (p.get("default"), why))
            elif p.get("required") == "true":
                self.r.warn("PARAM", p, None, "required parameter without default: every render must pass --param %s=..." % p.get("id"))
        for s in self.doc.iter("set"):
            p = self.ids.get(s.get("param", ""))
            if p is not None and p.tag == "param":
                why = self.param_value_ok(p, s.get("value", ""))
                if why:
                    self.r.error("PARAM", s, "value", "%r is %s" % (s.get("value"), why))
        for d in self.doc.iter("data"):
            if d.parent is None or d.parent.tag != "parameters":
                continue
            inline = (d.text or "").strip()
            if bool(inline) == bool(d.get("src")):
                self.r.error("STRUCTURE", d, "src", "data needs exactly one of inline rows or @src")
            if inline and d.get("format", "json") == "json":
                try:
                    rows = json.loads(inline)
                    if not isinstance(rows, list):
                        self.r.error("STRUCTURE", d, None, "inline JSON data must be an array of rows")
                except ValueError as ex:
                    self.r.error("STRUCTURE", d, None, "inline JSON does not parse: %s" % ex)

    # ------------------------------------------------------------ structure
    def node_window(self, n):
        """[start, end) in parent time. With startMarker/endMarker, @start/@end
        are offsets from the marker time (errata E10, as for key/@marker)."""
        s = fnum(n.get("start", "0"), 0.0)
        e = fnum(n.get("end")) if n.get("end") else None
        for key, is_start in (("startMarker", True), ("endMarker", False)):
            m = self.ids.get(n.get(key, ""))
            if m is None or m.tag not in ("marker", "#beat"):
                continue
            t = fnum(m.get("time"), 0.0) if m.tag == "marker" else self.beat_time(n.get(key))
            if is_start:
                s = t + (s or 0.0)
            else:
                e = t + (e or 0.0)
        return s, (e if e is not None else self.duration)

    def beat_time(self, ident):
        """Time of a generated beat.N / bar.N id (first beat grid, N from 0)."""
        g = next(iter(self.doc.iter("beatGrid")), None)
        if g is None:
            return 0.0
        kind, _, num = ident.partition(".")
        period = 60.0 / fnum(g.get("bpm"), 120)
        if kind == "bar":
            period *= int(fnum(g.get("beatsPerBar", "4"), 4))
        return fnum(g.get("offset", "0"), 0.0) + int(num) * period

    def top_level_timed(self, n):
        for a in n.ancestors():
            if a.tag in ("symbol", "sequence", "instance", "repeat"):
                return False
            if a.tag == "group" and (a.get("timeOffset") or a.get("timeScale")):
                return False
        return True

    def numbers(self, s):
        return [fnum(x) for x in re.split(r"[\s,]+", s.strip()) if x]

    def check_structure(self):
        r, D = self.r, self.duration
        for n in self.doc.iter():
            tag, g = n.tag, n.get
            if tag in NODE or tag == "camera":
                s, e = self.node_window(n)
                if e is not None and s is not None and e <= s:
                    r.error("TIME", n, "end", "end %g is not after start %g" % (e, s))
                elif self.top_level_timed(n) and s is not None and s >= D > 0:
                    r.warn("TIME", n, "start", "starts at %g, at or after the project end %g" % (s, D))
            if tag == "text" and n.parent is not None and n.parent.tag == "assets":
                spans = [c for c in n.children if c.tag == "span"]
                if (g("text") is not None) == bool(spans):
                    r.error("STRUCTURE", n, "text", "text content comes from @text or span children (exactly one)")
                if g("minSize") and g("maxSize") and fnum(g("minSize"), 0) > fnum(g("maxSize"), 0):
                    r.error("STRUCTURE", n, "minSize", "minSize > maxSize")
            if tag in ("shape", "vector") and g("shape") == "path" and not g("path"):
                r.error("STRUCTURE", n, "path", "shape=\"path\" needs @path")
            if tag == "vector" and g("shape") == "svg" and not g("src"):
                r.error("STRUCTURE", n, "src", "shape=\"svg\" needs @src")
            if tag in ("shape", "vector", "mask") and (g("shape") or g("type")) in ("polygon", "star") and int(fnum(g("points", "5"), 5)) < 3:
                r.error("STRUCTURE", n, "points", "polygons and stars need at least 3 points")
            if tag == "mask" and g("type") == "path" and not g("path"):
                r.error("STRUCTURE", n, "path", "type=\"path\" needs @path")
            for a in ("path", "emitterPath"):
                if g(a) and tag not in ("output", "poster", "thumbnail") and not value_fits("path", g(a)):
                    r.error("STRUCTURE", n, a, "not SVG path data")
            if g("cornerRadii") is not None:
                v = self.numbers(g("cornerRadii"))
                if len(v) != 4 or any(x is None or x < 0 for x in v):
                    r.error("STRUCTURE", n, "cornerRadii", "needs 4 non-negative numbers (TL TR BR BL)")
            if g("dash") is not None:
                v = self.numbers(g("dash"))
                if not v or any(x is None or x < 0 for x in v) or sum(x or 0 for x in v) == 0:
                    r.error("STRUCTURE", n, "dash", "needs non-negative lengths that are not all zero")
            if tag == "modifier":
                if g("type") == "corner-pin" and len(self.numbers(g("corners", ""))) != 8:
                    r.error("STRUCTURE", n, "corners", "corner-pin needs 8 numbers")
                if g("type") == "skin" and not g("skeleton"):
                    r.error("STRUCTURE", n, "skeleton", "skin needs @skeleton")
                rows, cols = int(fnum(g("rows", "4"), 4)), int(fnum(g("cols", "4"), 4))
                for p in n.children:
                    if p.tag == "point" and (int(fnum(p.get("row"), 0)) > rows or int(fnum(p.get("col"), 0)) > cols):
                        r.error("STRUCTURE", p, "row", "grid point outside the %dx%d warp grid" % (rows, cols))
            if tag == "meshGradient":
                rows, cols = int(fnum(g("rows", "2"), 2)), int(fnum(g("cols", "2"), 2))
                seen = set()
                for p in n.children:
                    if p.tag != "point":
                        continue
                    rc = (int(fnum(p.get("row"), 0)), int(fnum(p.get("col"), 0)))
                    if rc[0] >= rows or rc[1] >= cols:
                        r.error("STRUCTURE", p, "row", "point outside the %dx%d mesh" % (rows, cols))
                    if rc in seen:
                        r.error("STRUCTURE", p, "row", "point %s defined twice" % (rc,))
                    seen.add(rc)
                if len(seen) < rows * cols:
                    r.warn("STRUCTURE", n, None, "%d of %d mesh points defined" % (len(seen), rows * cols))
            if tag == "transformConstraint":
                t = g("type")
                if t == "follow-path" and not g("path"):
                    r.error("STRUCTURE", n, "path", "follow-path needs @path")
                elif t != "follow-path" and not g("target"):
                    r.error("STRUCTURE", n, "target", "type=%r needs @target" % t)
                if t == "track" and not g("point"):
                    r.error("STRUCTURE", n, "point", "track needs @point")
                if t == "distance" and not (g("minDistance") or g("maxDistance")):
                    r.error("STRUCTURE", n, "minDistance", "distance needs minDistance and/or maxDistance")
                if g("minDistance") and g("maxDistance") and fnum(g("minDistance"), 0) > fnum(g("maxDistance"), 0):
                    r.error("STRUCTURE", n, "minDistance", "minDistance > maxDistance")
            if tag == "constraint":
                if g("type") == "pin":
                    if g("b"):
                        r.error("STRUCTURE", n, "b", "pin ties body a to the world; b is not allowed")
                    if g("x") is None or g("y") is None:
                        r.error("STRUCTURE", n, "x", "pin needs @x and @y")
                elif not g("b"):
                    r.error("STRUCTURE", n, "b", "type=%r joins two bodies; @b is required" % g("type"))
                if g("minAngle") and g("maxAngle") and fnum(g("minAngle"), 0) > fnum(g("maxAngle"), 0):
                    r.error("STRUCTURE", n, "minAngle", "minAngle > maxAngle")
            if tag == "particleEmitter":
                if g("emitterShape") == "path" and not g("emitterPath"):
                    r.error("STRUCTURE", n, "emitterPath", "emitterShape=\"path\" needs @emitterPath")
                if g("emitterShape") == "asset-alpha" and not g("emitterAsset"):
                    r.error("STRUCTURE", n, "emitterAsset", "emitterShape=\"asset-alpha\" needs @emitterAsset")
                if g("shape") == "sprite" and not g("sprite"):
                    r.error("STRUCTURE", n, "sprite", "shape=\"sprite\" needs @sprite")
            if tag == "object3D":
                need = {"mesh": "mesh", "text": "text", "extrude": "path"}.get(g("primitive"))
                if need and not g(need):
                    r.error("STRUCTURE", n, need, "primitive=%r needs @%s" % (g("primitive"), need))
            if tag == "light":
                if g("type") == "dome" and not g("environment"):
                    r.error("STRUCTURE", n, "environment", "dome lights need @environment")
                if g("innerConeAngle") and fnum(g("innerConeAngle"), 0) > fnum(g("spotAngle", "45"), 45):
                    r.error("STRUCTURE", n, "innerConeAngle", "innerConeAngle exceeds spotAngle")
            if tag == "layer":
                asset = self.ids.get(g("asset", ""))
                if any(c.tag in ("textAnimator", "textPath") for c in n.children) and asset is not None and asset.tag != "text":
                    r.error("STRUCTURE", n, "asset", "textAnimator/textPath need a text asset")
                if any(c.tag == "timeRemap" for c in n.children) and (
                        g("speed", "1") != "1" or g("reverse", "false") != "false" or g("loop", "0") != "0"):
                    r.warn("STRUCTURE", n, None, "timeRemap replaces speed/reverse/loop, which are ignored")
                if g("cropLeft") and g("cropRight") and fnum(g("cropLeft"), 0) + fnum(g("cropRight"), 0) >= 1:
                    r.error("STRUCTURE", n, "cropLeft", "cropLeft + cropRight must be < 1")
                if g("cropTop") and g("cropBottom") and fnum(g("cropTop"), 0) + fnum(g("cropBottom"), 0) >= 1:
                    r.error("STRUCTURE", n, "cropTop", "cropTop + cropBottom must be < 1")
                if asset is not None and asset.tag in ("video",) and g("clipOut"):
                    if fnum(g("clipOut"), 0) > fnum(asset.get("duration"), 0) + 1e-9:
                        r.error("TIME", n, "clipOut", "clipOut beyond the video duration %s" % asset.get("duration"))
                if g("clipOut") and fnum(g("clipOut"), 0) <= fnum(g("clipIn", "0"), 0):
                    r.error("TIME", n, "clipOut", "clipOut must be after clipIn")
            if tag == "repeat" and (g("count") is None) == (g("over") is None):
                r.error("STRUCTURE", n, "count", "repeat needs exactly one of @count or @over")
            if tag == "imageSequence":
                if fnum(g("first"), 0) > fnum(g("last"), 0):
                    r.error("STRUCTURE", n, "first", "first > last")
                if not SEQ_PATTERN.search(g("src", "")):
                    r.error("STRUCTURE", n, "src", "src needs a printf pattern (%04d) or ####")
            if tag == "effect":
                t = g("type")
                for a in ("lift", "gamma", "gain", "slope", "offset", "power"):
                    if g(a) is not None and (len(self.numbers(g(a))) != 3 or None in self.numbers(g(a))):
                        r.error("STRUCTURE", n, a, "needs three numbers \"r,g,b\"")
                if t == "curves":
                    pts = [self.numbers(p) for p in (g("curve") or "").split()]
                    xs = [p[0] for p in pts if len(p) == 2]
                    if len(pts) < 2 or len(xs) != len(pts) or xs != sorted(xs) or any(x < 0 or x > 1 for x in xs):
                        r.error("STRUCTURE", n, "curve", "curves needs \"x,y x,y ...\" with increasing x in [0,1]")
                if t == "lut" and not g("src"):
                    r.error("STRUCTURE", n, "src", "lut needs @src")
                if t in ("displacement-map", "gradient-map", "difference-key") and not g("source") and not (t == "gradient-map" and g("paint")):
                    r.error("STRUCTURE", n, "source", "%s needs @source" % t)
                if t == "chroma-key" and not g("keyColor"):
                    r.error("STRUCTURE", n, "keyColor", "chroma-key needs @keyColor")
                if t == "levels" and fnum(g("inputBlack", "0"), 0) >= fnum(g("inputWhite", "1"), 1):
                    r.error("STRUCTURE", n, "inputBlack", "inputBlack must be below inputWhite")
            if tag == "transition":
                self.check_transition(n)
            if tag == "audioTrack":
                if g("clipOut") and fnum(g("clipOut"), 0) <= fnum(g("clipIn", "0"), 0):
                    r.error("TIME", n, "clipOut", "clipOut must be after clipIn")
                if fnum(g("start", "0"), 0) >= D > 0:
                    r.warn("TIME", n, "start", "track starts after the project end")
                if g("fitToDuration") == "true":
                    a = self.ids.get(g("asset", ""))
                    if a is not None and not a.get("bpm"):
                        r.error("STRUCTURE", n, "fitToDuration", "fitToDuration needs the asset's @bpm")
            if tag == "audioMix" and sum(1 for c in n.children if c.tag == "master") > 1:
                r.error("STRUCTURE", n, None, "at most one master bus")
            if tag == "captionTrack":
                self.check_captions(n)
            if tag == "safeArea":
                for a, b in (("top", "bottom"), ("left", "right")):
                    if fnum(g(a, "0"), 0) + fnum(g(b, "0"), 0) >= 1:
                        r.error("STRUCTURE", n, a, "%s + %s must be < 1" % (a, b))
            if tag == "layout" and g("aspect"):
                w, _, h = g("aspect").partition(":")
                if abs(int(w) / int(h) - fnum(g("width"), 1) / fnum(g("height"), 1)) > 0.01:
                    r.error("STRUCTURE", n, "aspect", "aspect %s does not match %sx%s" % (g("aspect"), g("width"), g("height")))
            if tag == "code":
                self.check_code(n)
            if tag == "chart":
                series = [c for c in n.children if c.tag == "series"]
                if not series and not g("src") and g("kind") not in ("counter", "progress"):
                    r.error("STRUCTURE", n, None, "chart needs series children or @src")
                lens = {len(c.get("values", "").split()) for c in series}
                if len(lens) > 1:
                    r.warn("STRUCTURE", n, None, "series have different lengths %s" % sorted(lens))
                if g("labels") and lens and len([x for x in re.split(r"[|,]", g("labels"))]) not in lens:
                    r.warn("STRUCTURE", n, "labels", "label count does not match the series length")
            if tag == "generated" and g("kind") in ("image", "video") and not (g("width") and g("height")):
                r.error("STRUCTURE", n, "width", "generated image/video needs @width and @height")
            if tag == "look" and not (g("src") or g("slope") or g("offset") or g("power") or g("saturation")):
                r.error("STRUCTURE", n, None, "look needs @src or CDL values")
            if tag == "accessibility" and g("requireCaptions") == "true" and not any(True for _ in self.doc.iter("captionTrack")):
                r.error("STRUCTURE", n, "requireCaptions", "requireCaptions is set but the document has no caption track")
            if tag == "project" and g("mode") == "viewport":
                s360 = next(iter(self.doc.iter("scene360")), None)
                if s360 is None or not s360.get("viewportCamera"):
                    if not any(True for _ in self.doc.iter("camera")):
                        r.error("STRUCTURE", n, "mode", "viewport mode needs a camera (scene360/@viewportCamera)")
        self.check_outputs()

    def check_transition(self, n):
        r = self.r
        f, t = self.ids.get(n.get("from", "")), self.ids.get(n.get("to", ""))
        if f is None and t is None:
            r.error("STRUCTURE", n, "from", "a transition needs @from and/or @to")
            return
        for x, a in ((f, "from"), (t, "to")):
            if x is not None and x.parent is not n.parent:
                r.error("STRUCTURE", n, a, "%r is not a sibling of the transition" % x.get("id"))
        if f is not None and t is not None and n.parent.tag != "sequence" and self.top_level_timed(n):
            fe, ts = self.node_window(f)[1], self.node_window(t)[0]
            if abs(fe - ts) > fnum(n.get("duration", "0.5"), 0.5):
                r.warn("TIME", n, None, "cut is undefined: %r ends at %g, %r starts at %g" % (f.get("id"), fe, t.get("id"), ts))
        if n.get("type") == "luma" and not n.get("matte"):
            r.error("STRUCTURE", n, "matte", "luma transitions need @matte")

    def check_captions(self, n):
        r = self.r
        cues = [c for c in n.children if c.tag == "cue"]
        if not cues and not n.get("src") and not n.get("transcribe"):
            r.error("STRUCTURE", n, None, "caption track needs cues, @src or @transcribe")
        if n.get("transcribe") and not (n.get("cache") and n.get("cacheSha256")):
            r.error("STRUCTURE", n, "cache", "transcribe needs @cache and @cacheSha256 (deterministic renders)")
        prev = None
        for c in cues:
            s, e = fnum(c.get("start"), 0), fnum(c.get("end"), 0)
            if e <= s:
                r.error("TIME", c, "end", "cue end must be after start")
            if not c.get("text") and not any(w.tag == "word" for w in c.children):
                r.error("STRUCTURE", c, "text", "cue needs @text or word children")
            if prev is not None and s < prev:
                r.warn("TIME", c, "start", "cue overlaps or precedes the previous cue")
            prev = e
            for w in c.children:
                ws, we = fnum(w.get("start"), 0), fnum(w.get("end"), 0)
                if we <= ws or ws < s - 1e-9 or we > e + 1e-9:
                    r.error("TIME", w, "start", "word must lie inside its cue and end after it starts")

    def check_code(self, n):
        k, d = n.get("kind"), n.get("data", "")

        def gs1_ok(digits):
            body, chk = digits[:-1], int(digits[-1])
            total = sum(int(c) * (3 if i % 2 == 0 else 1) for i, c in enumerate(reversed(body)))
            return (10 - total % 10) % 10 == chk
        if k in ("ean13", "upc-a"):
            full, short = (13, 12) if k == "ean13" else (12, 11)
            if not d.isdigit() or len(d) not in (full, short):
                self.r.error("STRUCTURE", n, "data", "%s needs %d or %d digits" % (k, short, full))
            elif len(d) == full and not gs1_ok(d):
                self.r.error("STRUCTURE", n, "data", "%s check digit is wrong" % k)
        elif k == "code128" and any(ord(c) > 127 for c in d):
            self.r.error("STRUCTURE", n, "data", "code128 encodes ASCII only")
        elif k == "qr" and len(d.encode()) > QR_CAPACITY[n.get("errorCorrection", "M")]:
            self.r.error("STRUCTURE", n, "data", "data exceeds QR capacity at level %s" % n.get("errorCorrection", "M"))

    def check_outputs(self):
        r, paths = self.r, {}
        for o in self.doc.children:
            if o.tag != "output":
                continue
            codec, cont, path = o.get("codec"), o.get("container"), o.get("path", "")
            allowed = CONTAINERS.get(codec, set())
            if cont and cont not in allowed:
                r.error("OUTPUT", o, "container", "%s cannot be muxed into %s (allowed: %s)" % (codec, cont, ", ".join(sorted(allowed)) or "none"))
            if codec == "audio-only" and not cont:
                r.error("OUTPUT", o, "container", "audio-only needs @container wav, m4a or mp3")
            if codec and codec.endswith("-sequence") and not SEQ_PATTERN.search(path):
                r.error("OUTPUT", o, "path", "%s needs a numbered path (%%04d or ####)" % codec)
            if o.get("alpha") == "true":
                if codec not in ALPHA_CODECS:
                    r.error("OUTPUT", o, "alpha", "%s cannot carry alpha" % codec)
                elif codec == "prores" and o.get("proresProfile") not in ("4444", "4444xq"):
                    r.error("OUTPUT", o, "proresProfile", "alpha ProRes needs proresProfile 4444 or 4444xq")
                elif codec == "gif":
                    r.warn("OUTPUT", o, "alpha", "GIF alpha is 1-bit")
            if o.get("proresProfile") and codec != "prores":
                r.warn("OUTPUT", o, "proresProfile", "ignored for codec %s" % codec)
            if o.get("transfer") in ("pq", "hlg") and "10" not in o.get("pixelFormat", "") and "12" not in o.get("pixelFormat", ""):
                r.error("OUTPUT", o, "pixelFormat", "HDR transfer %s needs a 10/12-bit pixelFormat" % o.get("transfer"))
            s, e = fnum(o.get("start", "0"), 0), fnum(o.get("end")) if o.get("end") else self.duration
            if e is not None and e <= s:
                r.error("TIME", o, "end", "output end must be after start")
            if e is not None and e > self.duration + 1e-9:
                r.error("TIME", o, "end", "output end %g is after the project end %g" % (e, self.duration))
            if path in paths:
                r.error("OUTPUT", o, "path", "path already written by the output at line %d" % paths[path].line)
            paths[path] = o
            burn = o.get("burnCaptions")
            if burn and o.get("captions") and burn in o.get("captions").split():
                r.warn("OUTPUT", o, "burnCaptions", "the same track is both burned and embedded")
            for c in o.children:
                if c.tag in ("poster", "thumbnail"):
                    t = fnum(c.get("time", "0"), 0)
                    if not c.get("marker") and not 0 <= t < self.duration:
                        r.error("TIME", c, "time", "still time %g is outside the project" % t)
                    fmt, ext = c.get("format", "jpeg"), os.path.splitext(c.get("path", ""))[1].lower().lstrip(".")
                    if ext and ext not in ({"jpeg": {"jpg", "jpeg"}}.get(fmt, {fmt})):
                        r.warn("OUTPUT", c, "path", "extension .%s does not match format %s" % (ext, fmt))
                if c.tag == "destination":
                    self.check_destination(c)

    def check_destination(self, d):
        uri, kind = d.get("uri", ""), d.get("kind")
        m = re.match(r"^([A-Za-z][A-Za-z0-9+.-]*):", uri)
        scheme = m.group(1).lower() if m else ""
        if scheme not in DEST_SCHEMES.get(kind, ()):
            self.r.error("DESTINATION", d, "uri", "scheme %r does not fit kind %s" % (scheme or "(none)", kind))
        if re.match(r"^[A-Za-z][A-Za-z0-9+.-]*://[^/@]*:[^/@]*@", uri):
            self.r.error("DESTINATION", d, "uri", "credentials in the URI; name an environment profile in @credentials")
        if re.search(r"(?i)[?&](sig|signature|x-amz-signature|token|key)=", uri):
            self.r.error("DESTINATION", d, "uri", "the URI carries a signature or token; use @credentials")
        if kind == "azure-blob" and ".blob.core.windows.net" not in uri:
            self.r.warn("DESTINATION", d, "uri", "not an Azure blob endpoint")

    # -------------------------------------------------------------- design
    def has_depth(self, n):
        return any(x.get("threeD") == "true" or x.tag in ("object3D",) or
                   (x.tag == "group" and x.get("collapse") == "true") for x in n.iter())

    def active_camera(self, cams, t):
        hit = None
        for c, s, e in cams:
            if s <= t < e:
                hit = c
        return hit

    def check_camera_cuts(self):
        cams = [(c,) + self.node_window(c) for c in self.doc.iter("camera")
                if c.get("active", "true") != "false" and self.top_level_timed(c)]
        if len(cams) < 2:
            return
        edges = sorted({t for _, s, e in cams for t in (s, e)})
        for tr in self.doc.iter("transition"):
            f, t = self.ids.get(tr.get("from", "")), self.ids.get(tr.get("to", ""))
            if not self.top_level_timed(tr) or not any(x is not None and self.has_depth(x) for x in (f, t)):
                continue
            if t is not None:
                cut = self.node_window(t)[0]
            elif f is not None:
                cut = self.node_window(f)[1]
            else:
                continue
            d = fnum(tr.get("duration", "0.5"), 0.5)
            lo, hi = {"start": (cut, cut + d), "end": (cut - d, cut)}.get(tr.get("alignment", "center"), (cut - d / 2, cut + d / 2))
            for b in edges:
                if lo < b < hi:
                    before, after = self.active_camera(cams, b - 1e-6), self.active_camera(cams, b + 1e-6)
                    if before is not after:
                        self.r.warn("CAMERA-CUT", tr, None,
                                    "camera switches from %s to %s at %g, inside the %g-%g transition; "
                                    "the outgoing 2.5D content jumps mid-transition"
                                    % (before.get("id") if before is not None else "none",
                                       after.get("id") if after is not None else "none", b, lo, hi))

    def check_isolation(self):
        for g in self.doc.iter():
            if g.tag not in ("group", "sequence") or g.get("collapse") != "true":
                continue
            why = "effects" if g.get("effects") else ("isolate" if g.get("isolate") == "true" else None)
            if why:
                self.r.warn("ISOLATION", g, why,
                            "@%s isolates the group, so its 2.5D children are flattened and collapse=\"true\" "
                            "has no effect; put the effects on the children" % why)

    DISPLAY_OF = {("rec709", "bt1886"): "rec709", ("rec709", "auto"): "rec709",
                  ("srgb", "srgb"): "srgb", ("srgb", "auto"): "srgb"}

    def check_display(self):
        cm = next(iter(self.doc.iter("colorManagement")), None)
        if cm is None or cm.get("ocioConfig"):
            return
        display = cm.get("display", "srgb").lower()
        names = {display, {"bt1886": "rec709", "bt709": "rec709", "rec.709": "rec709"}.get(display, display)}
        for o in (c for c in self.doc.children if c.tag == "output"):
            cs, tr = o.get("colorSpace", "srgb"), o.get("transfer", "auto")
            want = self.DISPLAY_OF.get((cs, tr), cs if tr in ("auto",) else "%s-%s" % (cs, tr))
            if not names & {want, cs}:
                self.r.warn("DISPLAY", cm, "display",
                            "display %r but output %r is %s/%s; the view transform targets the wrong display"
                            % (cm.get("display", "srgb"), o.get("id") or o.get("path"), cs, tr))

    def check_symbol_clip(self):
        for sym in self.doc.iter("symbol"):
            W, H = fnum(sym.get("width")), fnum(sym.get("height"))
            if not W or not H:
                continue
            for n in sym.children:
                if n.tag != "layer" or any(fnum(n.get(a, "0"), 0) for a in ("rotation", "skewX", "skewY")):
                    continue
                a = self.ids.get(n.get("asset", ""))
                vals = [fnum(n.get(k, d)) for k, d in (("x", "0"), ("y", "0"), ("anchorX", "0"), ("anchorY", "0"),
                                                         ("scaleX", "1"), ("scaleY", "1"))]
                if a is None or None in vals or not a.get("width") or not a.get("height"):
                    continue
                x, y, ax, ay, sx, sy = vals
                l, t = x - ax * sx, y - ay * sy
                r, b = l + fnum(a.get("width"), 0) * sx, t + fnum(a.get("height"), 0) * sy
                l, r, t, b = min(l, r), max(l, r), min(t, b), max(t, b)
                if l < 0 or t < 0 or r > W or b > H:
                    self.r.warn("SYMBOL-CLIP", n, None,
                                "spans %g,%g..%g,%g but symbol %r is %gx%g; the part outside its box is clipped"
                                % (l, t, r, b, sym.get("id"), W, H))

    def check_shared_seeds(self):
        uses = {}
        for inst in self.doc.iter("instance"):
            mult = 2 if any(a.tag == "repeat" for a in inst.ancestors()) else 1
            uses[inst.get("symbol")] = uses.get(inst.get("symbol"), 0) + mult
        for n in self.doc.iter():
            if "seed" not in n.attrib or n.tag == "project":
                continue
            owner = next((a for a in n.ancestors() if a.tag in ("symbol", "repeat")), None)
            if owner is None:
                continue
            if owner.tag == "repeat" or uses.get(owner.get("id"), 0) > 1:
                self.r.warn("SEED-SHARED", n, "seed",
                            "fixed seed inside %s %r: every copy gets the same random sequence and moves in "
                            "lockstep; omit @seed so it follows the scoped id"
                            % ("repeat" if owner.tag == "repeat" else "symbol", owner.get("id")))

    def check_design(self):
        self.check_camera_cuts()
        self.check_isolation()
        self.check_display()
        self.check_symbol_clip()
        self.check_shared_seeds()

    # ------------------------------------------------------------- version
    def check_version(self, v10):
        if self.version != "1.0" or v10 is None:
            return
        v10.assign_types(self.doc) if False else None
        # re-type the document with the 1.0 schema to find 1.1-only constructs
        stack = [(self.doc, v10.root_type)]
        while stack:
            n, key = stack.pop()
            if key is None:
                self.r.error("VERSION-GATE", n, None, "<%s> requires version=\"1.1\"" % n.tag)
                continue
            info = v10.info(key)
            for a, v in n.attrib.items():
                s = info["attrs"].get(a)
                if s is not None and s.family == "enum" and v not in s.enums:
                    self.r.error("VERSION-GATE", n, a, "value %r requires version=\"1.1\"" % v)
            for c in n.children:
                stack.append((c, info["children"].get(c.tag)))

    # --------------------------------------------------------------- files
    def local(self, uri):
        if re.match(r"^[A-Za-z][A-Za-z0-9+.-]+://", uri) and not uri.startswith("file://"):
            return None
        p = uri[7:] if uri.startswith("file://") else uri
        return p if os.path.isabs(p) else os.path.join(self.dir, p)

    def check_files_exist(self):
        for n in self.doc.iter():
            if n.tag in ("output", "poster", "thumbnail", "destination"):
                continue
            for a, v in n.attrib.items():
                s = self.attr_simple(n, a)
                if s is None or s.family != "uri" and not (n.tag == "imageSequence" and a == "src"):
                    continue
                p = self.local(v)
                if p is None:
                    continue
                if n.tag == "imageSequence":
                    self.check_sequence_files(n, p)
                    continue
                if not os.path.exists(p):
                    self.r.error("FILES", n, a, "file not found: %s" % p)
                    continue
                h = n.get(URI_ATTRS_HASH.get(a, "")) if a in URI_ATTRS_HASH else None
                if h and os.path.isfile(p):
                    got = hashlib.sha256(open(p, "rb").read()).hexdigest()
                    if got != h:
                        self.r.error("FILES", n, URI_ATTRS_HASH[a], "SHA-256 mismatch for %s (file is %s)" % (v, got))

    def check_sequence_files(self, n, pattern):
        first, last = int(fnum(n.get("first"), 0)), int(fnum(n.get("last"), 0))
        step = int(fnum(n.get("step", "1"), 1))

        def name(i):
            if "#" in pattern:
                m = re.search(r"#+", pattern)
                return pattern[:m.start()] + str(i).zfill(m.end() - m.start()) + pattern[m.end():]
            return pattern % i
        missing = [i for i in range(first, last + 1, max(step, 1)) if not os.path.exists(name(i))]
        if missing:
            lvl = self.r.error if n.get("missingFrame", "error") == "error" else self.r.warn
            lvl("FILES", n, "src", "%d of %d frames missing (first: %s)" % (len(missing), (last - first) // max(step, 1) + 1, name(missing[0])))

    # ----------------------------------------------------------------- run
    def run(self, v10):
        self.s.assign_types(self.doc)
        self.collect_ids()
        self.check_version(v10)
        self.check_refs()
        self.check_values()
        self.check_animation()
        self.check_conditions()
        self.check_overrides()
        self.check_params()
        self.check_structure()
        self.check_cycles()
        self.check_design()
        if self.check_files:
            self.check_files_exist()

# ---------------------------------------------------------------- XSD layer


def run_xsd(schema, scene):
    try:
        from lxml import etree as LX                      # in-process when available
    except ImportError:
        LX = None
    if LX is not None:
        parser = LX.XMLParser(no_network=True, resolve_entities=False)
        xsd = LX.XMLSchema(LX.parse(schema, parser))
        ok = xsd.validate(LX.parse(scene, parser))
        return ok, ["%s:%d: %s" % (scene, e.line, e.message) for e in xsd.error_log]
    cmd = ["xmllint", "--noout", "--nonet", "--schema", schema, scene]
    if shutil.which("xmllint") is None:
        if shutil.which("flatpak") is None:
            return None, "xmllint not found (install libxml2 or run inside the Flatpak SDK)"
        dirs = {os.path.dirname(os.path.abspath(p)) for p in (schema, scene)}
        cmd = ["flatpak", "run"] + ["--filesystem=%s" % d for d in sorted(dirs)] + \
              ["--command=xmllint", SDK, "--noout", "--nonet", "--schema",
               os.path.abspath(schema), os.path.abspath(scene)]
    p = subprocess.run(cmd, capture_output=True, text=True)
    lines = [l for l in p.stderr.splitlines() if l.strip() and not l.endswith(" validates")
             and "fails to validate" not in l]
    return p.returncode == 0, lines


def self_check(schema):
    """Every IDREF attribute of the schema must have a REF_KINDS rule."""
    missing = []
    for key in schema.all_types():
        for a, s in schema.info(key)["attrs"].items():
            if s.family in ("idref", "idrefs"):
                owners = [k for k, v in schema.info(key)["children"].items()]
                known = any(ref_kinds(tag, a)[0] for tag in [key.replace("Type", "").replace("el:", ""), "*"])
                if not known and not any((t, a) in REF_KINDS for t in element_names_of(schema, key)):
                    missing.append((key, a))
    return missing


def element_names_of(schema, key):
    names = set()
    for k in schema.all_types():
        for child, ck in schema.info(k)["children"].items():
            if ck == key:
                names.add(child)
    return names


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("scene", nargs="?", help="scene document to validate")
    ap.add_argument("--schema", default=DEFAULT_SCHEMA)
    ap.add_argument("--no-xsd", action="store_true", help="skip the xmllint layer")
    ap.add_argument("--check-files", action="store_true", help="check local files and SHA-256 values")
    ap.add_argument("--strict", action="store_true", help="treat warnings as errors")
    ap.add_argument("--json", action="store_true", help="print the report as JSON")
    ap.add_argument("--list-rules", action="store_true")
    args = ap.parse_args(argv)
    if args.list_rules:
        for k, v in RULES.items():
            print("%-14s %s" % (k, v))
        return 0
    if not args.scene:
        ap.error("a scene path is required")
    try:
        schema = Schema(args.schema)
    except (OSError, ET.ParseError) as ex:
        print("error: cannot read schema: %s" % ex, file=sys.stderr)
        return 2
    gaps = [(k, a) for k, a in self_check(schema)]
    if gaps:
        print("internal error: no reference-kind rule for %s" % ", ".join("%s@%s" % g for g in gaps), file=sys.stderr)
        return 2
    report = Report(args.scene)
    if not args.no_xsd:
        ok, lines = run_xsd(args.schema, args.scene)
        if ok is None:
            print("error: %s (use --no-xsd to skip)" % lines, file=sys.stderr)
            return 2
        for l in lines:
            m = re.match(r"^.*?:(\d+): (.*)$", l)
            report.items.append({"level": "error", "rule": "XSD", "line": int(m.group(1)) if m else 0,
                                 "where": "", "message": m.group(2) if m else l})
    try:
        doc = parse_document(args.scene, report)
    except (OSError, xml.parsers.expat.ExpatError) as ex:
        print("error: %s" % ex, file=sys.stderr)
        return 2
    v10 = Schema(V10_SCHEMA) if os.path.exists(V10_SCHEMA) else None
    Checker(doc, schema, report, args.scene, args.check_files).run(v10)
    report.items.sort(key=lambda i: (i["line"], i["rule"]))
    errors, warnings = report.count("error"), report.count("warning")
    valid = errors == 0 and (warnings == 0 or not args.strict)
    if args.json:
        print(json.dumps({"scene": args.scene, "valid": valid, "errors": errors,
                          "warnings": warnings, "items": report.items}, indent=2))
    else:
        for i in report.items:
            print("%s:%d: %s [%s] %s%s" % (args.scene, i["line"], i["level"], i["rule"],
                                            i["where"] + ": " if i["where"] else "", i["message"]))
        print("%s: %s (%d error%s, %d warning%s; layers: %s)" % (
            args.scene, "VALID" if valid else "INVALID", errors, "" if errors == 1 else "s",
            warnings, "" if warnings == 1 else "s", "semantic" if args.no_xsd else "xsd+semantic"))
    return 0 if valid else 1


if __name__ == "__main__":
    sys.exit(main())
