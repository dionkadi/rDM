// Design Tokens — JS mirror of the visual system.
//
// Source of truth is `src/app.css` (what the app actually renders).
// This file exists for JS-side access (tests, canvas drawing); when a
// value changes, change it in BOTH places. See docs/UI_DESIGN_PLAN.md.

export const tokens = {
  // ── Color (dark theme values; light theme lives in app.css) ─────
  color: {
    // Base
    bg: '#17191e',
    bgElevated: '#1e2128',
    bgHover: '#242830',
    bgActive: '#2a2f38',

    // Surfaces
    surface: '#1e2128',
    surfaceSolid: '#1e2128',
    surfaceHover: '#242830',
    surfaceActive: '#2a2f38',

    // Borders
    border: 'rgba(255, 255, 255, 0.08)',
    border2: 'rgba(255, 255, 255, 0.05)',
    borderStrong: 'rgba(255, 255, 255, 0.15)',
    borderFocus: 'rgba(65, 201, 162, 0.55)',

    track: '#2a2f38',

    // Text
    text: '#e6e8eb',
    textMuted: '#9ba1ab',
    textFaint: '#8c95a4',
    textInverse: '#0e1512',

    // Semantic — accent carries action + progress only
    primary: '#41c9a2',
    primaryHover: '#58d4b0',
    primaryDim: 'rgba(65, 201, 162, 0.12)',
    primaryStrong: 'rgba(65, 201, 162, 0.28)',

    success: '#54c68f',
    successDim: 'rgba(84, 198, 143, 0.12)',
    successStrong: 'rgba(84, 198, 143, 0.28)',

    warning: '#dfa84e',
    warningDim: 'rgba(223, 168, 78, 0.12)',
    warningStrong: 'rgba(223, 168, 78, 0.28)',

    danger: '#e5716a',
    dangerDim: 'rgba(229, 113, 106, 0.12)',
    dangerStrong: 'rgba(229, 113, 106, 0.28)',

    info: '#7ba3e8',
    infoDim: 'rgba(123, 163, 232, 0.12)',
    infoStrong: 'rgba(123, 163, 232, 0.28)',

    // Category hues (quiet, deliberately away from the semantic set)
    category: {
      video: '#b58ad8',
      audio: '#5fb6ae',
      archive: '#c3a363',
      document: '#8ca6c9',
      image: '#c98fa8',
      binary: '#98a2b3',
      code: '#8ca6c9',
      other: '#9ba1ab',
    },
  },

  // ── Typography (system stacks — zero network dependency) ──────
  font: {
    family: {
      display:
        'system-ui, -apple-system, "Segoe UI", "Noto Sans SC", "PingFang SC", "Microsoft YaHei", sans-serif',
      ui:
        'system-ui, -apple-system, "Segoe UI", "Noto Sans SC", "PingFang SC", "Microsoft YaHei", sans-serif',
      mono:
        'ui-monospace, "JetBrains Mono", "SF Mono", "Noto Sans Mono", Menlo, Consolas, monospace',
    },
    size: {
      xs: '11px',
      sm: '12px',
      base: '13px',
      lg: '14px',
      xl: '16px',
      '2xl': '20px',
    },
    weight: {
      light: '400',
      normal: '400',
      medium: '500',
      semibold: '600',
      bold: '700',
    },
    lineHeight: {
      tight: '1.15',
      snug: '1.25',
      normal: '1.5',
      relaxed: '1.6',
    },
    letterSpacing: {
      tight: '0',
      normal: '0',
      wide: '0.01em',
      wider: '0.02em',
      widest: '0.02em',
    },
  },

  // ── Spacing (4px base grid) ──────────────────────────────────
  space: {
    0: '0',
    1: '4px',
    2: '8px',
    3: '12px',
    4: '16px',
    5: '20px',
    6: '24px',
    7: '28px',
    8: '32px',
    10: '40px',
    12: '48px',
    16: '64px',
    20: '80px',
    24: '96px',
  },

  // ── Border radius (small; rows are square) ───────────────────
  radius: {
    xs: '3px',
    sm: '6px',
    md: '8px',
    lg: '12px',
    xl: '16px',
    full: '9999px',
  },

  // ── Shadows (floating layers only) ───────────────────────────
  shadow: {
    flat: 'none',
    raised: '0 1px 2px rgba(0, 0, 0, 0.28)',
    floating: '0 12px 32px -16px rgba(0, 0, 0, 0.55)',
    modal: '0 24px 60px -28px rgba(0, 0, 0, 0.7)',
    glow: '0 0 0 2px rgba(65, 201, 162, 0.12)',
  },

  // ── Motion (functional only) ─────────────────────────────────
  motion: {
    duration: {
      instant: '0ms',
      fast: '120ms',
      normal: '180ms',
    },
    easing: {
      linear: 'linear',
      easeOut: 'cubic-bezier(0.4, 0, 0.2, 1)',
      easeIn: 'cubic-bezier(0.4, 0, 1, 1)',
      easeInOut: 'cubic-bezier(0.4, 0, 0.2, 1)',
    },
  },

  // ── Breakpoints ──────────────────────────────────────────────
  breakpoint: {
    sm: '640px',
    md: '768px',
    lg: '900px',
    xl: '1280px',
    '2xl': '1400px',
  },

  // ── Z-Index ──────────────────────────────────────────────────
  zIndex: {
    base: '0',
    dropdown: '100',
    sticky: '200',
    overlay: '300',
    modal: '400',
    popover: '500',
    tooltip: '600',
    toast: '700',
  },

  // ── Sizing ───────────────────────────────────────────────────
  size: {
    topbar: '48px',
    filterbar: '38px',
    statusbar: '32px',
    rowHeight: '40px',
    rowHeightCompact: '34px',
  },
} as const;

// ── CSS Custom Property Generator ─────────────────────────────
export function generateCSSVariables(): string {
  const lines: string[] = [':root {'];

  function flatten(obj: Record<string, unknown>, prefix = ''): void {
    for (const [key, value] of Object.entries(obj)) {
      const name = `--${prefix}${key.replace(/([A-Z])/g, '-$1').toLowerCase()}`;
      if (typeof value === 'object' && value !== null) {
        flatten(value as Record<string, unknown>, `${prefix}${key}-`);
      } else {
        lines.push(`  ${name}: ${value};`);
      }
    }
  }

  flatten(tokens.color as Record<string, unknown>, 'color-');
  flatten(tokens.font.size as Record<string, unknown>, 'font-size-');
  flatten(tokens.space as Record<string, unknown>, 'space-');
  flatten(tokens.radius as Record<string, unknown>, 'radius-');
  flatten(tokens.shadow as Record<string, unknown>, 'shadow-');
  flatten(tokens.motion.duration as Record<string, unknown>, 'duration-');
  flatten(tokens.motion.easing as Record<string, unknown>, 'easing-');
  flatten(tokens.zIndex as Record<string, unknown>, 'z-');
  flatten(tokens.size as Record<string, unknown>, 'size-');

  lines.push(`  --font-display: ${tokens.font.family.display};`);
  lines.push(`  --font-ui: ${tokens.font.family.ui};`);
  lines.push(`  --font-mono: ${tokens.font.family.mono};`);
  lines.push(`  --bp-sm: ${tokens.breakpoint.sm};`);
  lines.push(`  --bp-md: ${tokens.breakpoint.md};`);
  lines.push(`  --bp-lg: ${tokens.breakpoint.lg};`);
  lines.push(`  --bp-xl: ${tokens.breakpoint.xl};`);
  lines.push(`  --bp-2xl: ${tokens.breakpoint['2xl']};`);

  lines.push('}');
  return lines.join('\n');
}

// ── Type-safe token access ─────────────────────────────────────
export type TokenPath =
  | `color.${keyof typeof tokens.color}`
  | `font.size.${keyof typeof tokens.font.size}`
  | `space.${keyof typeof tokens.space}`
  | `radius.${keyof typeof tokens.radius}`
  | `shadow.${keyof typeof tokens.shadow}`
  | `duration.${keyof typeof tokens.motion.duration}`
  | `easing.${keyof typeof tokens.motion.easing}`
  | `z.${keyof typeof tokens.zIndex}`
  | `size.${keyof typeof tokens.size}`;

export function getToken(path: TokenPath): string {
  const parts = path.split('.');
  let current: unknown = tokens;
  for (const part of parts) {
    if (current && typeof current === 'object' && part in current) {
      current = (current as Record<string, unknown>)[part];
    } else {
      return '';
    }
  }
  return String(current);
}
