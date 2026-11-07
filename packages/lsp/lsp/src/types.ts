/**
 * LSP seam vocabulary: the normalized request, provider, and result contracts. Types only — the
 * {@link LspError} taxonomy and the {@link LspProviderId} brand factory are runtime and live in
 * `index.ts`. Positions and ranges are zero-based UTF-16, matching the protocol; the model-facing
 * tool owns the one-based cursor convention. The seam exposes no protocol types, process or document
 * controls, or generic JSON-RPC escape hatch — the four semantic operations plus a provider-optional
 * diagnostics snapshot.
 * @module @deepseek-ai/dsh-lsp/types
 */

import type { LspProviderId } from './brand.ts'

/**
 * The four semantic queries the seam and model expose. A closed union: adding an operation is a
 * compile-enforced change across the seam, providers, and the tool. Symbols and call hierarchy are
 * not operations here; they need different schemas.
 */
export type LspOperation = 'goToDefinition' | 'findReferences' | 'goToImplementation' | 'hover'

/** A zero-based UTF-16 cursor coordinate, matching the LSP wire convention. */
export interface LspPosition {
  /** Zero-based line. */
  readonly line: number
  /** Zero-based UTF-16 code-unit offset within the line. */
  readonly character: number
}

/** A zero-based UTF-16 half-open range `[start, end)`. */
export interface LspRange {
  readonly start: LspPosition
  readonly end: LspPosition
}

/**
 * A caller's normalized query. Every field is required: `workspaceRoot` is caller-supplied,
 * `languageId` comes from the provider registration (not here), and consumers own timeouts and
 * result limits — so no field needs implementation defaulting and there is no `resolve()` step.
 */
export interface LspQueryRequest {
  /** Which semantic query to run. */
  readonly operation: LspOperation
  /** The source file to query (relative to `workspaceRoot` or absolute; the provider canonicalizes). */
  readonly filePath: string
  /** The zero-based UTF-16 cursor position to query at. */
  readonly position: LspPosition
  /** The workspace root the provider resolves against and indexes; required, never defaulted. */
  readonly workspaceRoot: string
}

/**
 * A request as a provider receives it: the caller's {@link LspQueryRequest} plus the `languageId`
 * the seam derived from the provider's extension mapping. The language id only synchronizes the
 * transient document; it does not participate in selection.
 */
export interface LspProviderQuery extends LspQueryRequest {
  /** The LSP language id for `filePath`, from this provider's extension mapping. */
  readonly languageId: string
}

/**
 * A request for one file's diagnostics. Providers open the document, wait for
 * the server to publish for that URI, and return the result; a server that
 * never publishes within the budget yields an empty list rather than failing.
 */
export interface LspDiagnoseRequest {
  /** The source file to diagnose (relative to `workspaceRoot` or absolute). */
  readonly filePath: string
  /** The workspace root the provider resolves against and indexes. */
  readonly workspaceRoot: string
  /** Longest wait for the server's publish after `didOpen`, in milliseconds. */
  readonly timeoutMs?: number
}

/** One resolved location: a document URI and the range within it. */
export interface LspLocation {
  /** The target document URI (`file:` or otherwise), verbatim from the server. */
  readonly uri: string
  /** The range within the target document. */
  readonly range: LspRange
}

/** Normalized hover content, or `null` for no hover at the position. */
export interface LspHover {
  /** The normalized hover text (markdown or plaintext, provider-joined). */
  readonly contents: string
  /** The range the hover applies to, when the server supplied one. */
  readonly range?: LspRange
}

/** Normalized diagnostic severity, matching the LSP numeric values 1–4. */
export type LspDiagnosticSeverity = 'error' | 'warning' | 'information' | 'hint'

/** One diagnostics item published by a configured language server. URIs stay
 * verbatim from the server; a consumer that needs workspace-relative paths
 * resolves them through the provider's canonical workspace URI.
 */
export interface LspDiagnostic {
  /** Document URI the diagnostic belongs to. */
  readonly uri: string
  /** Range the diagnostic covers. */
  readonly range: LspRange
  /** Normalized severity. */
  readonly severity: LspDiagnosticSeverity
  /** Human-readable message. */
  readonly message: string
  /** Server-declared source label, when present. */
  readonly source?: string
  /** Server-declared diagnostic code, when present. */
  readonly code?: string
}

/**
 * The closed result union. Navigation operations (`goToDefinition`, `findReferences`,
 * `goToImplementation`) normalize to `locations`; `hover` normalizes to content or `null`.
 * Consumers `switch` on `kind` to exhaustiveness so a new arm breaks compilation until handled.
 *
 * The `locations` variant carries `resolvedWorkspaceUri`: the provider's canonical `file:` URI for
 * the request's workspace root. A caller that relativizes location URIs MUST use this, not parse the
 * request's possibly symlinked process path with host-platform rules; the execution platform may
 * differ from the caller's.
 */
export type LspQueryResult =
  | { readonly kind: 'locations'; readonly locations: readonly LspLocation[]; readonly resolvedWorkspaceUri: string }
  | { readonly kind: 'hover'; readonly hover: LspHover | null }

/**
 * A language-server backend registered on `ctx.lsp`. Each provider owns a stable {@link
 * LspProviderId} and an extension-to-language-id map (lowercase, leading-dot keys).
 * `findReferences` always includes declarations — the provider enforces this internally; callers
 * get no flag.
 */
export interface LspProvider {
  /** Stable provider identity, reserved atomically with the extension mappings. */
  readonly id: LspProviderId
  /** Lowercase leading-dot extension → LSP language id (e.g. `{ '.ts': 'typescript' }`). */
  readonly extensionToLanguage: Readonly<Record<string, string>>
  /**
   * Run one query. The seam has already selected this provider and derived `languageId`.
   * @param request - the resolved provider query (caller request + derived language id).
   * @param signal - optional cancellation; the provider stops its own work when it aborts.
   * @returns the normalized, closed-union result.
   */
  query(request: LspProviderQuery, signal?: AbortSignal): Promise<LspQueryResult>
  /**
   * Diagnostics last published by this provider's servers for one workspace.
   *
   * Providers that cannot observe diagnostics omit this method; the seam then
   * reports none for that provider. Servers only publish after a document they
   * know about changes, so an empty result does not mean the workspace is clean.
   * @param workspaceRoot - the same caller-supplied root used by queries.
   * @returns normalized diagnostics, bounded by the provider.
   */
  diagnostics?(workspaceRoot: string): Promise<readonly LspDiagnostic[]>
  /**
   * Open one file, wait for the server to publish diagnostics for it, and
   * return them. Providers that keep no server state omit this method.
   * @param request - file, workspace root, and the wait budget.
   * @param signal - optional cancellation for the whole open/wait/close cycle.
   * @returns normalized diagnostics for the file; empty when the server did not publish in time.
   */
  diagnose?(request: LspDiagnoseRequest, signal?: AbortSignal): Promise<readonly LspDiagnostic[]>
}

/**
 * The LSP capability seam (`ctx.lsp`). Owns provider registration/selection and normalized query
 * execution; exposes exactly the four operations and no protocol escape hatch. Diagnostics are a
 * separate provider-optional snapshot, never a query result.
 */
export interface LspService {
  /**
   * Register a provider, atomically reserving its id and every normalized extension. Any conflict
   * or invalid input publishes nothing and throws `LspError`; the returned disposer releases all
   * reservations. Disposed with the calling fiber.
   * @param provider - the backend to register.
   * @returns a synchronous disposer releasing the id and all extension reservations.
   */
  registerProvider(provider: LspProvider): () => void
  /**
   * Select a provider by the file's extension and run one query. Selection is per-query and
   * order-independent; no match throws `LspError` `LSP_UNAVAILABLE`.
   * @param request - the normalized query.
   * @param signal - optional cancellation forwarded to the selected provider.
   * @returns the normalized, closed-union result.
   */
  query(request: LspQueryRequest, signal?: AbortSignal): Promise<LspQueryResult>
  /**
   * Last diagnostics published by every configured provider for one workspace.
   *
   * Only registered providers contribute, so a server the user did not
   * configure or install can never appear here. Results are a snapshot, not a
   * stream; callers poll when their panel needs freshness.
   * @param workspaceRoot - canonical or caller-supplied workspace root.
   * @returns merged diagnostics in provider registration order.
   */
  diagnostics(workspaceRoot: string): Promise<readonly LspDiagnostic[]>
  /**
   * Ask the provider that handles this file's extension to open it and wait
   * for diagnostics. A file with no registered provider throws `LSP_UNAVAILABLE`.
   * @param request - file, workspace root, and optional wait budget.
   * @param signal - optional cancellation forwarded to the provider.
   * @returns normalized diagnostics for the file.
   */
  diagnoseFile(request: LspDiagnoseRequest, signal?: AbortSignal): Promise<readonly LspDiagnostic[]>
}
