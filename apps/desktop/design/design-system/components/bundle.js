/* @ds-bundle: {"format":4,"namespace":"TmpNam","components":[{"name":"Button"},{"name":"Icon"},{"name":"Tag"},{"name":"StatusDot"},{"name":"Spinner"},{"name":"ProgressBar"},{"name":"Banner"},{"name":"TextField"},{"name":"Checkbox"},{"name":"Radio"},{"name":"PopupButton"},{"name":"Menu"},{"name":"SegmentedControl"},{"name":"SizePicker"},{"name":"GainControl"},{"name":"StageList"},{"name":"Sheet"},{"name":"Sidebar"},{"name":"UnitStatus"},{"name":"ActivityCard"},{"name":"Toolbar"},{"name":"ConnectSteps"}]} */
(function () {
  var React = window.React;
  var h = React.createElement;
  var useState = React.useState;
  var useEffect = React.useEffect;

  function cx() {
    var out = [];
    for (var i = 0; i < arguments.length; i++) if (arguments[i]) out.push(arguments[i]);
    return out.join(' ');
  }
  function omit(obj, keys) {
    var o = {};
    for (var k in obj) if (Object.prototype.hasOwnProperty.call(obj, k) && keys.indexOf(k) < 0) o[k] = obj[k];
    return o;
  }
  // Controlled when the prop is given, otherwise keeps its own state.
  function useMaybeControlled(value, fallback) {
    var s = useState(value === undefined ? fallback : value);
    useEffect(function () { if (value !== undefined) s[1](value); }, [value]);
    return s;
  }

  var ICONS = {
    captures: '<path d="M1.5 8h2l2-5 3 10 2-7 1.2 2h2.8"/>',
    tone3000: '<path d="M4.5 12.5a3 3 0 0 1-.3-6 4 4 0 0 1 7.7 1 2.5 2.5 0 0 1 .3 5"/><path d="M8 8v6m-2-2 2 2 2-2"/>',
    sdcard: '<path d="M5.5 1.5h6v13h-8V3.5z"/><path d="M7 4v2m2-2v2"/>',
    settings: '<path d="M2 4.5h7m3 0h2M2 11.5h2m3 0h7"/><circle cx="10.5" cy="4.5" r="1.5"/><circle cx="5.5" cy="11.5" r="1.5"/>',
    warning: '<path d="M8 2 14.5 13.5h-13z"/><path d="M8 6.5v3M8 11.6v.1"/>',
    error: '<circle cx="8" cy="8" r="6.5"/><path d="M8 4.5v4M8 11v.1"/>',
    info: '<circle cx="8" cy="8" r="6.5"/><path d="M8 7.5v4M8 5v.1"/>',
    usb: '<path d="M6 1.5v3.5m4-3.5v3.5M4.5 5h7v3a3.5 3.5 0 0 1-7 0zM8 11.5v3"/>',
    add: '<path d="M8 3v10M3 8h10"/>',
    minus: '<path d="M3 8h10"/>',
    check: '<path d="M3 8.5 6.5 12 13 4.5"/>',
    close: '<path d="M4 4l8 8M12 4 4 12"/>',
    external: '<path d="M9 2.5h4.5V7M13.5 2.5 7.5 8.5M11.5 9.5v4h-9v-9h4"/>',
    bookmark: '<path d="M4.5 2h7v12L8 11l-3.5 3z"/>',
    refresh: '<path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5v3h-3"/>',
    lock: '<rect x="3.5" y="7" width="9" height="7" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/>',
    chevron: '<path d="M4.5 6.5 8 10l3.5-3.5"/>'
  };

  function Icon(props) {
    var name = props.name, size = props.size || 16;
    return h('svg', {
      className: cx('tn-icon', props.className), viewBox: '0 0 16 16', width: size, height: size,
      fill: 'none', stroke: 'currentColor', strokeWidth: props.strokeWidth || 1.5,
      strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': props['aria-label'] ? undefined : true,
      role: props['aria-label'] ? 'img' : undefined, 'aria-label': props['aria-label'], style: props.style,
      dangerouslySetInnerHTML: { __html: ICONS[name] || '' }
    });
  }
  Icon.names = Object.keys(ICONS);

  function Button(props) {
    var variant = props.variant || 'secondary', size = props.size || 'md';
    var rest = omit(props, ['variant', 'size', 'icon', 'iconOnly', 'state', 'className', 'children', 'type', 'destructiveText']);
    // Icon-only: a text child becomes the accessible name and tooltip.
    if (props.iconOnly && typeof props.children === 'string') {
      if (!rest['aria-label']) rest['aria-label'] = props.children;
      if (!rest.title) rest.title = props.children;
    }
    return h('button', Object.assign({
      type: props.type || 'button',
      className: cx('tn-btn', 'tn-btn-' + variant, size !== 'md' && 'tn-btn-' + size, props.iconOnly && 'tn-btn-icon',
        props.destructiveText && 'tn-btn-danger-text', props.state && 'is-' + props.state, props.className)
    }, rest),
      props.icon ? h(Icon, { name: props.icon }) : null,
      props.iconOnly ? null : props.children);
  }

  function Tag(props) {
    var tone = props.tone || 'neutral';
    return h('span', { className: cx('tn-tag', tone !== 'neutral' && 'tn-tag-' + tone, props.className), style: props.style }, props.children);
  }

  function StatusDot(props) {
    var dot = h('span', { className: cx('tn-dot', 'tn-dot-' + (props.tone || 'off')), 'aria-hidden': true });
    if (!props.label) return dot;
    return h('span', { className: 'tn-status' }, dot, props.label);
  }

  function Spinner(props) {
    return h('span', { className: cx('tn-spinner', props.size === 'lg' && 'tn-spinner-lg'), role: 'progressbar', 'aria-label': props.label || 'Working' });
  }

  function ProgressBar(props) {
    var indet = props.indeterminate || props.value == null;
    var tone = props.tone && props.tone !== 'accent' ? 'tn-bar-' + props.tone : null;
    var bar = h('div', {
      className: cx('tn-bar', props.size === 'lg' && 'tn-bar-lg', tone, indet && 'tn-bar-indet'),
      role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': indet ? undefined : props.value,
      'aria-label': props.label || 'Progress'
    }, h('i', { style: indet ? null : { width: Math.max(0, Math.min(100, props.value)) + '%' } }));
    if (!props.label && !props.detail) return bar;
    return h('div', { className: 'tn-progress' },
      h('div', { className: 'tn-progress-head' }, h('span', null, props.label), h('span', { className: 'tn-progress-detail' }, props.detail)),
      bar);
  }

  var BANNER_ICON = { info: 'info', note: 'info', warn: 'warning', error: 'error', ok: 'check' };
  function Banner(props) {
    var tone = props.tone || 'info';
    return h('div', { className: cx('tn-banner', 'tn-banner-' + tone, props.className), role: tone === 'error' || tone === 'warn' ? 'alert' : 'status', style: props.style },
      h(Icon, { name: BANNER_ICON[tone] }),
      h('div', { className: 'tn-banner-body' },
        h('div', null,
          props.title ? h('div', { className: 'tn-banner-title' }, props.title) : null,
          props.children),
        props.actions && props.actions.length ? h('div', { className: 'tn-banner-actions' }, props.actions.map(function (a, i) {
          return h(Button, { key: i, size: 'sm', variant: a.variant || 'secondary', onClick: a.onClick, disabled: a.disabled }, a.label);
        })) : null),
      props.onDismiss ? h(Button, { variant: 'plain', size: 'sm', onClick: props.onDismiss }, props.dismissLabel || 'Dismiss') : null);
  }

  var fieldSeq = 0;
  function TextField(props) {
    var idState = useState(function () { return props.id || 'tn-field-' + (++fieldSeq); });
    var id = idState[0];
    var input = h('input', Object.assign({
      id: id, className: cx('tn-input', props.mono && 'is-mono', props.error && 'is-error', props.state && 'is-' + props.state),
      'aria-invalid': props.error ? true : undefined
    }, omit(props, ['id', 'label', 'mono', 'error', 'help', 'state', 'children', 'className', 'style'])));
    return h('div', { className: cx('tn-field', props.className), style: props.style },
      props.label ? h('label', { className: 'tn-field-label', htmlFor: id }, props.label) : null,
      props.children ? h('div', { className: 'tn-field-row' }, input, props.children) : input,
      props.error && props.error !== true ? h('span', { className: 'tn-error' }, props.error) : null,
      props.help ? h('span', { className: 'tn-help' }, props.help) : null);
  }

  function Checkbox(props) {
    return h('label', { className: cx('tn-check', props.disabled && 'is-disabled', props.className), style: props.style },
      h('input', { type: 'checkbox', checked: props.checked, defaultChecked: props.defaultChecked, disabled: props.disabled, onChange: props.onChange, 'aria-label': props.label ? undefined : props['aria-label'] }),
      props.label ? h('span', null, props.label) : null);
  }

  function Radio(props) {
    return h('label', { className: cx('tn-check', props.row && 'tn-choice', props.row && props.checked && 'is-selected', props.disabled && 'is-disabled', props.className), style: props.style },
      h('input', { type: 'radio', name: props.name, checked: props.checked, defaultChecked: props.defaultChecked, disabled: props.disabled, onChange: props.onChange }),
      h('span', { className: 'tn-check-body' },
        h('span', { style: { fontWeight: props.detail ? 500 : 400 } }, props.label),
        props.detail ? h('span', { className: 'tn-check-detail' }, props.detail) : null),
      props.note ? h('span', { className: 'tn-check-note' }, props.note) : null);
  }

  function PopupButton(props) {
    return h('button', {
      type: 'button', className: cx('tn-popup', props.tone === 'warn' && 'tn-popup-warn', props.open && 'is-open', props.className),
      onClick: props.onClick, disabled: props.disabled, 'aria-haspopup': 'listbox', 'aria-expanded': !!props.open, style: props.style
    }, h('span', null, props.children, props.marked ? h('span', { className: 'tn-popup-mark' }, ' •') : null), h(Icon, { name: 'chevron' }));
  }

  function Menu(props) {
    var items = props.items || [];
    return h('div', { className: cx('tn-menu', props.className), role: 'listbox', 'aria-label': props['aria-label'], style: Object.assign({ width: props.width }, props.style) },
      items.map(function (it, i) {
        if (it.type === 'header') return h('div', { key: i, className: 'tn-menu-head' }, it.label);
        if (it.type === 'separator') return h('div', { key: i, className: 'tn-menu-sep', role: 'separator' });
        return h('button', {
          key: i, type: 'button', role: 'option', 'aria-selected': !!it.checked, disabled: it.disabled,
          className: cx('tn-menu-item', it.active && 'is-active', it.link && 'is-link'),
          onClick: function () { if (it.onClick) it.onClick(); if (props.onSelect) props.onSelect(it.value != null ? it.value : it.label); }
        }, h('span', { className: 'tn-menu-check' }, it.checked ? h(Icon, { name: 'check', size: 12 }) : null), it.label,
          it.note ? h('span', { className: 'tn-menu-note' }, it.note) : null);
      }));
  }

  function SegmentedControl(props) {
    var opts = (props.options || []).map(function (o) { return typeof o === 'string' ? { label: o, value: o } : o; });
    var st = useMaybeControlled(props.value, opts.length ? opts[0].value : null);
    return h('div', { className: 'tn-seg', role: 'tablist', 'aria-label': props['aria-label'] },
      opts.map(function (o) {
        var on = o.value === st[0];
        return h('button', { key: o.value, type: 'button', role: 'tab', 'aria-selected': on, className: on ? 'is-on' : '',
          onClick: function () { st[1](o.value); if (props.onChange) props.onChange(o.value); } }, o.label);
      }));
  }

  function SizePicker(props) {
    var sizes = (props.sizes || ['Feather', 'Full']).map(function (s) { return typeof s === 'string' ? { label: s } : s; });
    var st = useMaybeControlled(props.value, sizes[sizes.length - 1].label);
    var n = sizes.length;
    return h('div', { className: cx('tn-sizepick', props.disabled && 'is-disabled'), role: 'radiogroup', 'aria-label': props['aria-label'] || 'Size the player loads' },
      sizes.map(function (s, i) {
        var on = s.label === st[0];
        var note = s.note || (i === 0 ? 'smallest' : i === n - 1 ? 'largest' : '');
        return h('button', { key: s.label, type: 'button', role: 'radio', 'aria-checked': on, className: on ? 'is-on' : '',
          onClick: function () { st[1](s.label); if (props.onChange) props.onChange(s.label); } },
          s.label, note ? h('small', null, note) : null);
      }));
  }

  function gainDb(x) {
    if (x <= 0) return 'Muted';
    var db = 20 * Math.log10(x);
    var r = Math.round(db * 10) / 10;
    return (r > 0 ? '+' : r < 0 ? '−' : '±') + Math.abs(r).toFixed(1) + ' dB';
  }
  function fmtX(x) { return (Math.round(x * 10) / 10).toString() + '×'; }
  function GainControl(props) {
    var st = useMaybeControlled(props.value, 1);
    var v = st[0];
    function set(x) {
      x = Math.max(0, Math.min(8, Math.round(x * 2) / 2));
      st[1](x); if (props.onChange) props.onChange(x);
    }
    var showHelp = props.help !== false;
    return h('div', { className: cx('tn-gain', props.disabled && 'is-disabled') },
      h('div', { className: 'tn-gain-top' },
        h('span', { className: 'tn-field-label' }, props.label || 'Output gain'),
        h('div', { className: 'tn-stepper' },
          h('button', { type: 'button', 'aria-label': 'Decrease gain', onClick: function () { set(v - 0.5); } }, '−'),
          h('span', { className: 'tn-stepper-value', 'aria-live': 'polite' }, fmtX(v)),
          h('button', { type: 'button', 'aria-label': 'Increase gain', onClick: function () { set(v + 0.5); } }, '+'))),
      h('input', { className: 'tn-range', type: 'range', min: 0, max: 8, step: 0.5, value: v, 'aria-label': props.label || 'Output gain',
        onChange: function (e) { set(parseFloat(e.target.value)); } }),
      h('div', { className: 'tn-ticks', 'aria-hidden': true }, ['0×', '2×', '4×', '6×', '8×'].map(function (t) { return h('span', { key: t }, t); })),
      h('div', { className: 'tn-gain-foot' },
        h('span', { className: 'tn-gain-db' }, gainDb(v) + (props.previous != null && props.previous !== v ? ' · was ' + fmtX(props.previous) : '')),
        h('span', { className: 'tn-gain-presets' },
          h(Button, { size: 'sm', onClick: function () { set(1); } }, '1× as captured'),
          h(Button, { size: 'sm', onClick: function () { set(4); } }, '4×'))),
      showHelp ? h('span', { className: 'tn-help' }, 'NAM captures play about 12 dB quieter than the stock amp blocks. 4× makes up for it.') : null);
  }

  function StageList(props) {
    return h('ul', { className: 'tn-stages' }, (props.stages || []).map(function (s, i) {
      var state = s.state || 'todo';
      var ic = state === 'done' ? h(Icon, { name: 'check', strokeWidth: 2.2 }) : state === 'fail' ? h(Icon, { name: 'close', strokeWidth: 2.2 }) : state === 'now' ? h(Spinner, null) : null;
      return h('li', { key: i, className: 'tn-stage-' + state },
        h('span', { className: 'tn-stage-ic' }, ic), h('span', null, s.label), h('span', { className: 'tn-stage-detail' }, s.detail || ''));
    }));
  }

  function Sheet(props) {
    if (props.open === false) return null;
    var actions = props.actions || [];
    return h('div', { className: 'tn-scrim', onClick: function (e) { if (e.target === e.currentTarget && props.onClose) props.onClose(); } },
      h('div', { className: 'tn-sheet', role: props.alert ? 'alertdialog' : 'dialog', 'aria-label': props.title, style: { width: props.width || 480 } },
        h('div', { className: 'tn-sheet-body' },
          props.title ? h('h2', { className: 'tn-sheet-title' }, props.title) : null,
          props.children),
        actions.length || props.extra ? h('div', { className: 'tn-sheet-foot' },
          props.extra ? h(Button, { variant: 'plain', className: 'tn-sheet-extra', onClick: props.extra.onClick, disabled: props.extra.disabled }, props.extra.label) : null,
          actions.map(function (a, i) { return h(Button, { key: i, variant: a.variant || 'secondary', onClick: a.onClick, disabled: a.disabled, autoFocus: a.autoFocus }, a.label); })) : null));
  }

  var UNIT = {
    connected: ['ok', 'Connected · NAM card'],
    looking: ['spin', 'Looking for the unit…'],
    missing: ['off', 'Not connected'],
    busy: ['warn', 'Engine restarting']
  };
  function UnitStatus(props) {
    var u = UNIT[props.state || 'missing'];
    var mark = u[0] === 'spin' ? h(Spinner, { label: 'Looking for the unit' }) : h(StatusDot, { tone: u[0] });
    if (props.compact) return h('div', { className: 'tn-unit', title: 'Tone Master Pro · ' + u[1] }, mark);
    return h('div', { className: 'tn-unit' },
      h('span', { className: 'tn-unit-name' }, 'Tone Master Pro'),
      h('span', { className: 'tn-status' }, mark, u[1]));
  }

  function ActivityCard(props) {
    return h('div', { className: 'tn-activity', 'aria-live': 'polite' },
      h('span', { className: 'tn-activity-title' }, props.title),
      props.detail ? h('span', { className: 'tn-activity-detail' }, props.detail) : null,
      h(ProgressBar, { value: props.value, indeterminate: props.value == null || props.value < 0, label: props.title }));
  }

  var NAV = [['captures', 'Captures', 'captures'], ['tone3000', 'Tone3000', 'tone3000'], ['sdcard', 'SD Card', 'sdcard']];
  function Sidebar(props) {
    var active = props.active || 'captures';
    function item(key, label, icon, count) {
      return h('button', {
        key: key, type: 'button', className: cx('tn-nav-item', active === key && 'is-on'), 'aria-current': active === key ? 'page' : undefined,
        'aria-label': props.compact ? label : undefined,
        onClick: function () { if (props.onNavigate) props.onNavigate(key); }
      }, h(Icon, { name: icon }), props.compact ? null : label,
        !props.compact && count != null && count !== '' ? h('span', { className: 'tn-nav-count' }, count) : null);
    }
    var act = props.activity;
    return h('nav', { className: cx('tn-sidebar', props.compact && 'is-compact'), 'aria-label': 'Sections', style: props.style },
      h('div', { className: 'tn-lights', 'aria-hidden': true }, h('span'), h('span'), h('span')),
      h('div', { className: 'tn-nav' }, NAV.map(function (n) { return item(n[0], n[1], n[2], n[0] === 'captures' ? props.captureCount : null); })),
      h('div', { className: 'tn-sidebar-fill' }),
      act && !props.compact ? h(ActivityCard, { title: act.title, detail: act.detail, value: act.value }) : null,
      h('div', { className: 'tn-nav', style: { paddingBottom: 8 } }, item('settings', 'Settings', 'settings')),
      h(UnitStatus, { state: props.unit || 'missing', compact: props.compact }));
  }

  function Toolbar(props) {
    return h('header', { className: 'tn-toolbar', style: props.style },
      h('h1', { className: 'tn-toolbar-title' }, props.title),
      props.subtitle ? h('span', { className: 'tn-toolbar-sub' }, props.subtitle) : null,
      props.children ? h('div', { className: 'tn-toolbar-actions' }, props.children) : null);
  }

  function ConnectSteps(props) {
    var steps = props.steps || ['Insert the NAM SD card and power the unit on.', 'Wait for the preset screen.', "Plug the unit's USB-C cable into this Mac."];
    return h('ol', { className: 'tn-steps' }, steps.map(function (s, i) { return h('li', { key: i }, s); }));
  }

  var api = {
    Button: Button, Icon: Icon, Tag: Tag, StatusDot: StatusDot, Spinner: Spinner, ProgressBar: ProgressBar, Banner: Banner,
    TextField: TextField, Checkbox: Checkbox, Radio: Radio, PopupButton: PopupButton, Menu: Menu, SegmentedControl: SegmentedControl,
    SizePicker: SizePicker, GainControl: GainControl, StageList: StageList, Sheet: Sheet, Sidebar: Sidebar, UnitStatus: UnitStatus,
    ActivityCard: ActivityCard, Toolbar: Toolbar, ConnectSteps: ConnectSteps,
    utils: { gainDb: gainDb }
  };
  window.TmpNam = Object.assign(window.TmpNam || {}, api);
})();
