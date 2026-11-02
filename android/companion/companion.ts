#!/usr/bin/env bun
// Hex companion: one small, private, authenticated service for the things a phone
// can't do alone. It stores the portable profile as one ETag-versioned document, so
// devices sync through compare-and-swap, and forwards long-text polish to the OpenCode
// service on this machine, so provider credentials never reach the phone.
//
// It binds to loopback only. `tailscale serve` is the only way in, which gives
// devices on your tailnet HTTPS with a real certificate and leaves the internet out.
// It never logs request bodies. See README.md.
import { createHash, randomBytes, timingSafeEqual } from "node:crypto"
import { chmodSync, closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, readdirSync, renameSync, statSync, writeSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

const MAX_PROFILE_BYTES = 1024 * 1024
const MAX_GENERATE_BYTES = 512 * 1024
const MAX_DEADLINE_MS = 60_000
const GENERATE_ROUTES = ["/api/experimental/generate", "/api/generate"]

export interface Config {
  directory: string
  stateDirectory: string
}

export const defaultConfig = (): Config => ({
  directory: process.env.HEX_COMPANION_DIR ?? join(process.env.XDG_DATA_HOME ?? join(homedir(), ".local/share"), "hex-companion"),
  stateDirectory: join(process.env.XDG_STATE_HOME ?? join(homedir(), ".local/state"), "opencode"),
})

const sha256 = (data: string | Uint8Array) => createHash("sha256").update(data).digest("hex")

const ensurePrivate = (directory: string) => {
  mkdirSync(directory, { recursive: true, mode: 0o700 })
  chmodSync(directory, 0o700)
}

/** Replaces a file so a crash leaves the old or the new content, never a mix. */
const writeAtomic = (path: string, data: Uint8Array) => {
  const temporary = `${path}.${process.pid}.tmp`
  const fd = openSync(temporary, "w", 0o600)
  try {
    writeSync(fd, data)
    fsyncSync(fd)
  } finally {
    closeSync(fd)
  }
  renameSync(temporary, path)
}

// ---- tokens: only hashes are kept; revoke a device by deleting its line ----

const tokensPath = (config: Config) => join(config.directory, "tokens")

export const addDevice = (config: Config, name: string): string => {
  ensurePrivate(config.directory)
  if (!/^[\w.-]{1,40}$/.test(name)) throw new Error("Device names are letters, numbers, dot, dash, or underscore")
  const token = randomBytes(32).toString("base64url")
  const existing = existsSync(tokensPath(config)) ? readFileSync(tokensPath(config), "utf8") : ""
  writeAtomic(tokensPath(config), new TextEncoder().encode(`${existing}${sha256(token)} ${name}\n`))
  return token
}

const authorized = (config: Config, request: Request): boolean => {
  const header = request.headers.get("authorization") ?? ""
  if (!header.startsWith("Bearer ") || !existsSync(tokensPath(config))) return false
  const presented = Buffer.from(sha256(header.slice(7)))
  let ok = false
  for (const line of readFileSync(tokensPath(config), "utf8").split("\n")) {
    const known = Buffer.from(line.split(" ")[0] ?? "")
    if (known.length === presented.length && timingSafeEqual(known, presented)) ok = true
  }
  return ok
}

// ---- profile: one document, written only by compare-and-swap ----

const profilePath = (config: Config) => join(config.directory, "profile.json")

const quoted = (etag: string) => `"${etag}"`

const getProfile = (config: Config): Response => {
  if (!existsSync(profilePath(config))) return new Response(null, { status: 404 })
  const body = readFileSync(profilePath(config))
  return new Response(body, { headers: { ETag: quoted(sha256(body)), "Content-Type": "application/json" } })
}

const putProfile = async (config: Config, request: Request): Promise<Response> => {
  const declared = Number(request.headers.get("content-length") ?? 0)
  if (declared > MAX_PROFILE_BYTES) return new Response("Too large", { status: 413 })
  const body = new Uint8Array(await request.arrayBuffer())
  if (body.length > MAX_PROFILE_BYTES) return new Response("Too large", { status: 413 })
  try {
    const parsed = JSON.parse(new TextDecoder().decode(body))
    if (parsed.schema !== "hex.profile" || typeof parsed.version !== "number") throw new Error()
  } catch {
    return new Response("Not a Hex profile", { status: 422 })
  }
  ensurePrivate(config.directory)
  const exists = existsSync(profilePath(config))
  const current = exists ? quoted(sha256(readFileSync(profilePath(config)))) : null
  const ifMatch = request.headers.get("if-match")
  const ifNoneMatch = request.headers.get("if-none-match")
  if (ifMatch === null && ifNoneMatch === null) return new Response("A version precondition is required", { status: 428 })
  if ((ifNoneMatch === "*" && exists) || (ifMatch !== null && ifMatch !== current)) {
    return new Response("The profile changed", { status: 412 })
  }
  writeAtomic(profilePath(config), body)
  return new Response(null, { status: exists ? 204 : 201, headers: { ETag: quoted(sha256(body)) } })
}

// ---- polish: forwards to the OpenCode service on this machine ----

interface Registration {
  id: string
  url: string
  pid: number
  password?: string
}

const loopback = (url: string) => {
  try {
    const { protocol, hostname } = new URL(url)
    return protocol === "http:" && (hostname === "127.0.0.1" || hostname === "localhost" || hostname === "[::1]")
  } catch {
    return false
  }
}

const authHeader = (password?: string): Record<string, string> =>
  password ? { Authorization: `Basic ${btoa(`opencode:${password}`)}` } : {}

/** Reads the owner-only registration and confirms the service answers, as desktop Hex does. */
const findOpenCode = async (config: Config): Promise<Registration | null> => {
  const paths = [join(config.stateDirectory, "service.json")]
  if (existsSync(config.stateDirectory)) {
    for (const name of readdirSync(config.stateDirectory).sort()) {
      if (name.startsWith("service-") && name.endsWith(".json")) paths.push(join(config.stateDirectory, name))
    }
  }
  for (const path of paths) {
    if (!existsSync(path)) continue
    try {
      if ((statSync(path).mode & 0o077) !== 0) continue
      const registration: Registration = JSON.parse(readFileSync(path, "utf8"))
      if (!loopback(registration.url)) continue
      const info = await fetch(`${registration.url}/api/info`, { headers: authHeader(registration.password), signal: AbortSignal.timeout(1000) })
      if (info.ok && (await info.json()).pid === registration.pid) return registration
    } catch {}
  }
  return null
}

const generate = async (config: Config, request: Request): Promise<Response> => {
  const body = new Uint8Array(await request.arrayBuffer())
  if (body.length > MAX_GENERATE_BYTES) return new Response("Too large", { status: 413 })
  const deadline = Math.min(Number(request.headers.get("x-hex-deadline-ms")) || MAX_DEADLINE_MS, MAX_DEADLINE_MS)
  const service = await findOpenCode(config)
  if (!service) return Response.json({ _tag: "CompanionError", reason: "opencode_not_running", message: "OpenCode isn't running on the companion" }, { status: 503 })
  const signal = AbortSignal.any([request.signal, AbortSignal.timeout(deadline)])
  try {
    for (const route of GENERATE_ROUTES) {
      const reply = await fetch(`${service.url}${route}`, {
        method: "POST",
        headers: { "Content-Type": "application/json", ...authHeader(service.password) },
        body,
        signal,
      })
      if (reply.status === 404) continue
      // The body passes through untouched, so the client reads OpenCode's own typed errors.
      return new Response(await reply.text(), { status: reply.status, headers: { "Content-Type": "application/json" } })
    }
    return Response.json({ _tag: "CompanionError", reason: "no_route", message: "OpenCode has no generation route" }, { status: 502 })
  } catch {
    return Response.json({ _tag: "CompanionError", reason: "upstream", message: "OpenCode didn't answer in time" }, { status: 504 })
  }
}

// ---- server ----

export const handler = (config: Config) => async (request: Request): Promise<Response> => {
  const { pathname } = new URL(request.url)
  if (!authorized(config, request)) return new Response("Unauthorized", { status: 401, headers: { "WWW-Authenticate": "Bearer" } })
  if (pathname === "/v1/profile" && request.method === "GET") return getProfile(config)
  if (pathname === "/v1/profile" && request.method === "PUT") return putProfile(config, request)
  if (pathname === "/v1/generate" && request.method === "POST") return generate(config, request)
  if (pathname === "/v1/health" && request.method === "GET") {
    return Response.json({ ok: true, profile: existsSync(profilePath(config)), opencode: (await findOpenCode(config)) !== null })
  }
  return new Response("Not found", { status: 404 })
}

if (import.meta.main) {
  const config = defaultConfig()
  const [command, argument] = process.argv.slice(2)
  if (command === "add-device") {
    const token = addDevice(config, argument ?? "device")
    console.log(`Token for ${argument ?? "device"}. It won't be shown again:\n${token}`)
  } else if (command === "serve" || command === undefined) {
    ensurePrivate(config.directory)
    const server = Bun.serve({ hostname: "127.0.0.1", port: Number(process.env.HEX_COMPANION_PORT ?? 8787), maxRequestBodySize: 2 * MAX_PROFILE_BYTES, fetch: handler(config) })
    console.log(`Hex companion on http://${server.hostname}:${server.port}`)
  } else {
    console.error("usage: companion.ts [serve | add-device <name>]")
    process.exit(2)
  }
}
