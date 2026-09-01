// Design Tokens — Centralized design system for DM
// All values are CSS custom property compatible

export const tokens = {
  // ── Color Primitives ──────────────────────────────────────────────
  color: {
    // Base
    bg: '#0b0d12',
    bgElevated: '#0e1118',
    bgHover: '#12151e',
    bgActive: '#171b26',

    // Surfaces
    surface: 'rgba(255, 255, 255, 0.022)',
    surfaceSolid: '#12151e',
    surfaceHover: 'rgba(255, 255, 255, 0.035)',
    surfaceActive: 'rgba(255, 255, 255, 0.05)',

    // Borders
    border: 'rgba(255, 255, 255, 0.08)',
    borderStrong: 'rgba(255, 255, 255, 0.14)',
    borderFocus: 'rgba(122, 240, 200, 0.55)',

    // Text
    text: '#e9e7e2',
    textMuted: '#8b91a0',
    textFaint: '#565d6e',
    textInverse: '#06140f',

    // Semantic
    primary: '#7af0c8',
    primaryHover: '#b6f7e0',
    primaryDim: 'rgba(122, 240, 200, 0.12)',
    primaryStrong: 'rgba(122, 240, 200, 0.28)',

    success: '#74e0a4',
    successDim: 'rgba(116, 224, 164, 0.12)',
    successStrong: 'rgba(116, 224, 164, 0.28)',

    warning: '#f4b860',
    warningDim: 'rgba(244, 184, 96, 0.1)',
    warningStrong: 'rgba(244, 184, 96, 0.25)',

    danger: '#ff7a8f',
    dangerDim: 'rgba(255, 122, 143, 0.1)',
    dangerStrong: 'rgba(255, 122, 143, 0.25)',

    info: '#7aa2ff',
    infoDim: 'rgba(122, 162, 255, 0.1)',
    infoStrong: 'rgba(122, 162, 255, 0.25)',

    // Category hues
    category: {
      video: '#f4b860',
      audio: '#5ad1c4',
      archive: '#7af0c8',
      document: '#7aa2ff',
      image: '#b69cff',
      binary: '#ff9d6b',
      code: '#a5d6ff',
      other: '#9aa6c4',
    },
  },

  // ── Typography ────────────────────────────────────────────────────
  font: {
    family: {
      display: '"Fraunces", "Iowan Old Style", Georgia, serif',
      ui: '"Sora", system-ui, -apple-system, "Segoe UI", sans-serif',
      mono: '"JetBrains Mono", ui-monospace, "SF Mono", Menlo, monospace',
    },
    size: {
      xs: '11px',
      sm: '12.5px',
      base: '14px',
      lg: '15.5px',
      xl: '17px',
      '2xl': '19px',
      '3xl': '22px',
      '4xl': '30px',
      '5xl': '40px',
    },
    weight: {
      light: '300',
      normal: '400',
      medium: '500',
      semibold: '600',
      bold: '700',
    },
    lineHeight: {
      tight: '1.05',
      snug: '1.2',
      normal: '1.5',
      relaxed: '1.65',
    },
    letterSpacing: {
      tight: '-0.02em',
      normal: '0',
      wide: '0.02em',
      wider: '0.08em',
      widest: '0.18em',
    },
  },

  // ── Spacing (4px base grid) ──────────────────────────────────────
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

  // ── Border Radius ────────────────────────────────────────────────
  radius: {
    xs: '4px',
    sm: '8px',
    md: '12px',
    lg: '16px',
    xl: '24px',
    '2xl': '32px',
    full: '9999px',
  },

  // ── Shadows / Elevation ──────────────────────────────────────────
  shadow: {
    flat: 'none',
    raised: '0 4px 16px -8px rgba(0, 0, 0, 0.6)',
    floating: '0 12px 32px -16px rgba(0, 0, 0, 0.7)',
    modal: '0 24px 70px -28px rgba(0, 0, 0, 0.85)',
    glow: '0 0 0 3px rgba(122, 240, 200, 0.22)',
    glowDanger: '0 0 0 3px rgba(255, 122, 143, 0.22)',
    inner: 'inset 0 1px 0 rgba(255, 255, 255, 0.05)',
  },

  // ── Motion ───────────────────────────────────────────────────────
  motion: {
    duration: {
      instant: '0ms',
      fast: '120ms',
      normal: '180ms',
      slow: '280ms',
      slower: '400ms',
    },
    easing: {
      linear: 'linear',
      easeOut: 'cubic-bezier(0.4, 0, 0.2, 1)',
      easeIn: 'cubic-bezier(0.4, 0, 1, 1)',
      easeInOut: 'cubic-bezier(0.4, 0, 0.2, 1)',
      spring: 'cubic-bezier(0.34, 1.56, 0.64, 1)',
      springGentle: 'cubic-bezier(0.25, 1.2, 0.5, 1)',
    },
    stagger: '40ms',
  },

  // ── Breakpoints ──────────────────────────────────────────────────
  breakpoint: {
    sm: '640px',
    md: '768px',
    lg: '1024px',
    xl: '1280px',
    '2xl': '1400px',
  },

  // ── Z-Index ──────────────────────────────────────────────────────
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

  // ── Sizing ───────────────────────────────────────────────────────
  size: {
    sidebar: {
      expanded: '280px',
      collapsed: '64px',
    },
    topbar: '48px',
    toolbar: '44px',
    statusbar: '32px',
    rowHeight: '72px',
    rowHeightCompact: '60px',
  },
} as const;

// ── CSS Custom Property Generator ──────────────────────────────────
export function generateCSSVariables(): string {
  const lines: string[] = [':root {'];

  // Flatten tokens to CSS variables
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

  // Font families
  lines.push(`  --font-display: ${tokens.font.family.display};`);
  lines.push(`  --font-ui: ${tokens.font.family.ui};`);
  lines.push(`  --font-mono: ${tokens.font.family.mono};`);

  // Motion stagger
  lines.push(`  --stagger: ${tokens.motion.stagger};`);

  // Breakpoints (for JS use)
  lines.push(`  --bp-sm: ${tokens.breakpoint.sm};`);
  lines.push(`  --bp-md: ${tokens.breakpoint.md};`);
  lines.push(`  --bp-lg: ${tokens.breakpoint.lg};`);
  lines.push(`  --bp-xl: ${tokens.breakpoint.xl};`);
  lines.push(`  --bp-2xl: ${tokens.breakpoint['2xl']};`);

  lines.push('}');
  return lines.join('\n');
}

// ── Type-safe token access ─────────────────────────────────────────
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