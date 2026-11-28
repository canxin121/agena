import { MCP_APP_MAX_HEIGHT, type McpAppMode } from './bridge'

export function mcpAppContainerDimensions(mode: McpAppMode, width: number, height: number) {
  return {
    ...(width > 0 && Number.isFinite(width) ? { width } : {}),
    ...(mode === 'inline'
      ? { maxHeight: MCP_APP_MAX_HEIGHT }
      : height > 0 && Number.isFinite(height)
        ? { height }
        : {}),
  }
}

const colors = {
  '--color-background-primary': '--background',
  '--color-background-secondary': '--card',
  '--color-background-tertiary': '--muted',
  '--color-background-inverse': '--foreground',
  '--color-text-primary': '--foreground',
  '--color-text-secondary': '--muted-foreground',
  '--color-text-inverse': '--background',
  '--color-text-danger': '--destructive',
  '--color-border-primary': '--border',
  '--color-border-secondary': '--input',
  '--color-ring-primary': '--ring',
  '--color-ring-danger': '--destructive',
}

const metrics = {
  '--font-sans': '--font-sans',
  '--font-mono': '--font-mono',
  '--font-weight-normal': '--ui-regular-font-weight',
  '--font-text-xs-size': '--text-micro',
  '--font-text-sm-size': '--text-ui-label',
  '--font-text-md-size': '--text-markdown',
  '--border-radius-md': '--radius',
}

export function mcpAppStyleVariables(style: Pick<CSSStyleDeclaration, 'getPropertyValue'>) {
  const variables: Record<string, string> = {}
  for (const [name, token] of Object.entries(colors)) {
    const value = style.getPropertyValue(token).trim()
    if (value) variables[name] = `oklch(${value})`
  }
  for (const [name, token] of Object.entries(metrics)) {
    const value = style.getPropertyValue(token).trim()
    if (value) variables[name] = value
  }
  return variables
}
