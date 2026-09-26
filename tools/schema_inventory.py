# SPDX-License-Identifier: Apache-2.0
"""Expand the XSD's groups into element-context and attribute inventories.

This reads repository-owned schemas only. The generated C table is used by
both capability diagnostics and the generated feature matrix.
"""
import xml.etree.ElementTree as ET

XS = '{http://www.w3.org/2001/XMLSchema}'


class Schema:
    def __init__(self, path):
        self.root = ET.parse(path).getroot()
        self.definitions = {kind: {node.get('name'): node for node in
                                  self.root.findall(XS + kind)}
                            for kind in ('complexType', 'simpleType',
                                         'attributeGroup', 'group')}
        self.types = dict(self.definitions['complexType'])
        self.types['scene'] = self.root.find(XS + 'element/' + XS + 'complexType')
        self.children = {}
        self.attributes = {}
        for name, node in self.types.items():
            self.children[name] = {child.get('name'): child.get('type')
                                   for child in self.expand(node, 'element')}
            self.attributes[name] = {attr.get('name'): attr
                                     for attr in self.expand(node, 'attribute')}

    def expand(self, node, kind):
        for child in node:
            tag = child.tag.removeprefix(XS)
            if tag == kind:
                yield child
            elif tag in ('attributeGroup', 'group'):
                yield from self.expand(self.definitions[tag][child.get('ref')], kind)
            elif tag in ('sequence', 'choice', 'all', 'simpleContent',
                         'complexContent', 'extension'):
                if tag == 'extension' and child.get('base') in self.types:
                    yield from self.expand(self.types[child.get('base')], kind)
                yield from self.expand(child, kind)

    def values(self, attr):
        if attr is None:
            return []
        if attr.get('fixed') is not None:
            return [attr.get('fixed')]
        node = self.definitions['simpleType'].get(attr.get('type'), attr)
        return [enum.get('value') for enum in node.iter(XS + 'enumeration')]


def inventory(old, new, profile):
    """Rows: kind, host type, name, value/child type, version, implemented."""
    rows = []
    for host in sorted(new.types):
        for name, child in sorted(new.children[host].items()):
            key = host + '/' + name
            legacy = name in old.children.get(host, {})
            rows.append(('element', host, name, child, 10 if legacy else 11,
                         legacy or key in profile['elements']))
        for key, limit in profile.get('occurrence_limits', {}).items():
            owner, name = key.split('/')
            if owner == host:
                rows.append(('occurrence', host, name, str(limit), 10, False))
        for name, attr in sorted(new.attributes[host].items()):
            key = host + '/@' + name
            previous = old.attributes.get(host, {}).get(name)
            implemented = previous is not None or key in profile['attributes']
            rows.append(('attribute', host, name, attr.get('type', ''), 10,
                         implemented))
            values = new.values(attr)
            previous_values = old.values(previous)
            for value in values:
                # New attributes are permitted in 1.0; extensions of an
                # existing enum vocabulary require 1.1.
                legacy = previous is not None and (not previous_values or
                                                   value in previous_values)
                if host == 'scene' and name == 'version':
                    legacy = True
                minimum = 11 if previous_values and not legacy else 10
                rows.append(('value', host, name, value, minimum,
                             legacy or value in profile['values'].get(key, [])))
            vocabulary = profile.get('vocabularies', {}).get(key, {})
            if vocabulary and values:
                raise ValueError(f'semantic vocabulary duplicates XSD enum: {key}')
            for value, state in vocabulary.items():
                if state['version'] not in (10, 11) or type(state['implemented']) is not bool:
                    raise ValueError(f'invalid semantic vocabulary: {key}={value}')
                rows.append(('value', host, name, value, state['version'],
                             state['implemented']))
            forms = []
            if (attr.get('type') in ('lengthType', 'positiveLengthType') or
                    key == 'keyType/@value'):
                forms += ['relative-length']
            if attr.get('type') in ('colorType', 'paintType') or key == 'keyType/@value':
                forms += ['token']
            if attr.get('type') == 'paintType' or key == 'keyType/@value':
                forms += ['paint-reference']
            for form in forms:
                minimum = 11 if form == 'relative-length' and previous is not None else 10
                rows.append(('form', host, name, form, minimum,
                             form in profile['forms'].get(key, [])))
    keys = {kind: set() for kind in ('element', 'attribute', 'value', 'form')}
    for kind, host, name, value, _, _ in rows:
        if kind in keys:
            key = host + ('/' if kind == 'element' else '/@') + name
            keys[kind].add((key, value) if kind in ('value', 'form') else key)
    for kind in ('element', 'attribute'):
        unknown = set(profile[kind + 's']) - keys[kind]
        if unknown:
            raise ValueError(f'unknown {kind} capabilities: {sorted(unknown)}')
    for kind in ('value', 'form'):
        for key, values in profile[kind + 's'].items():
            for value in values:
                if (key, value) not in keys[kind]:
                    raise ValueError(f'unknown {kind} capability: {key}={value}')
    for key in profile.get('vocabularies', {}):
        if key not in keys['attribute']:
            raise ValueError(f'unknown semantic vocabulary attribute: {key}')
    for key, limit in profile.get('occurrence_limits', {}).items():
        host, name = key.split('/')
        if name not in new.children.get(host, {}) or not isinstance(limit, int) or limit < 1:
            raise ValueError(f'invalid occurrence capability: {key}={limit}')
    return sorted(rows, key=lambda row: (row[1], row[0], row[2], row[3]))
