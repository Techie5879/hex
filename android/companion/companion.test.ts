import { afterEach, beforeEach, describe, expect, test } from "bun:test"
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { addDevice, handler, type Config } from "./companion"

let root: string
let config: Config
let token: string
const profile = (extra = {}) => JSON.stringify({ schema: "hex.profile", version: 1, vocabulary: {}, settings: {}, conflicts: {}, ...extra })

const call = (path: string, init: RequestInit & { auth?: string | null } = {}) => {
  const headers = new Headers(init.headers)
  const auth = init.auth === undefined ? token : init.auth
  if (auth) headers.set("Authorization", `Bearer ${auth}`)
  return handler(config)(new Request(`http://companion${path}`, { ...init, headers }))
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "hex-companion-"))
  config = { directory: join(root, "data"), stateDirectory: join(root, "opencode") }
  token = addDevice(config, "phone")
})
afterEach(() => rmSync(root, { recursive: true, force: true }))

describe("authentication", () => {
  test("every route refuses a missing or wrong token", async () => {
    for (const path of ["/v1/profile", "/v1/health", "/nope"]) {
      expect((await call(path, { auth: null })).status).toBe(401)
      expect((await call(path, { auth: "wrong" })).status).toBe(401)
    }
    expect((await call("/v1/generate", { method: "POST", auth: null, body: "{}" })).status).toBe(401)
  })

  test("tokens are stored only as hashes in owner-only files", () => {
    const stored = readFileSync(join(config.directory, "tokens"), "utf8")
    expect(stored).not.toContain(token)
    expect(statSync(join(config.directory, "tokens")).mode & 0o077).toBe(0)
    expect(statSync(config.directory).mode & 0o077).toBe(0)
  })

  test("a revoked device stops working while others keep going", async () => {
    const other = addDevice(config, "mac")
    const file = join(config.directory, "tokens")
    writeFileSync(file, readFileSync(file, "utf8").split("\n").filter((l) => !l.endsWith(" phone")).join("\n"))
    expect((await call("/v1/profile")).status).toBe(401)
    expect((await call("/v1/profile", { auth: other })).status).toBe(404)
  })
})

describe("profile", () => {
  const put = (body: string, headers: Record<string, string>) => call("/v1/profile", { method: "PUT", body, headers })

  test("compare-and-swap: create once, then only from the current version", async () => {
    expect((await call("/v1/profile")).status).toBe(404)
    const created = await put(profile(), { "If-None-Match": "*" })
    expect(created.status).toBe(201)
    const etag = created.headers.get("etag")!
    expect((await put(profile({ a: 1 }), { "If-None-Match": "*" })).status).toBe(412)
    const fetched = await call("/v1/profile")
    expect(fetched.headers.get("etag")).toBe(etag)
    expect(await fetched.text()).toBe(profile())
    const updated = await put(profile({ a: 2 }), { "If-Match": etag })
    expect(updated.status).toBe(204)
    // The racer holding the old version loses, and nothing changes.
    expect((await put(profile({ a: 3 }), { "If-Match": etag })).status).toBe(412)
    expect(JSON.parse(await (await call("/v1/profile")).text()).a).toBe(2)
  })

  test("a write without a precondition or with the wrong shape is refused", async () => {
    expect((await put(profile(), {})).status).toBe(428)
    expect((await put("{}", { "If-None-Match": "*" })).status).toBe(422)
    expect((await put("nonsense", { "If-None-Match": "*" })).status).toBe(422)
    expect((await put(profile({ big: "x".repeat(1024 * 1024) }), { "If-None-Match": "*" })).status).toBe(413)
    expect(existsSync(join(config.directory, "profile.json"))).toBe(false)
  })
})

describe("generate", () => {
  const request = JSON.stringify({ prompt: "p", model: { providerID: "opencode-go", id: "m", variant: "none" } })
  let upstream: ReturnType<typeof Bun.serve> | undefined
  let seen: { path: string; auth: string | null; body: string }[]

  const serveOpenCode = (reply: (path: string) => Response | Promise<Response>, password = "secret") => {
    seen = []
    upstream = Bun.serve({
      port: 0,
      hostname: "127.0.0.1",
      async fetch(r) {
        const path = new URL(r.url).pathname
        if (path === "/api/info") return Response.json({ pid: 4242 })
        seen.push({ path, auth: r.headers.get("authorization"), body: await r.text() })
        return reply(path)
      },
    })
    mkdirSync(config.stateDirectory, { recursive: true })
    const file = join(config.stateDirectory, "service.json")
    writeFileSync(file, JSON.stringify({ id: "x", url: `http://127.0.0.1:${upstream.port}`, pid: 4242, password }))
    chmodSync(file, 0o600)
  }
  afterEach(() => upstream?.stop(true))

  const post = (headers: Record<string, string> = {}) => call("/v1/generate", { method: "POST", body: request, headers })

  test("forwards the request with its own credentials and returns the reply untouched", async () => {
    serveOpenCode(() => Response.json({ data: { text: "Clean." } }))
    const reply = await post()
    expect(reply.status).toBe(200)
    expect(await reply.json()).toEqual({ data: { text: "Clean." } })
    expect(seen[0]!.body).toBe(request)
    expect(seen[0]!.auth).toBe(`Basic ${btoa("opencode:secret")}`)
    expect(seen[0]!.auth).not.toContain(token)
  })

  test("OpenCode's typed quota error reaches the client exactly as sent", async () => {
    const quota = { _tag: "ProviderRequestError", providerID: "opencode-go", reason: "rate_limit", status: 402, message: "x" }
    serveOpenCode(() => Response.json(quota, { status: 502 }))
    const reply = await post()
    expect(reply.status).toBe(502)
    expect(await reply.json()).toEqual(quota)
  })

  test("an older OpenCode's route is tried only when the newer one is missing", async () => {
    serveOpenCode((path) => (path.includes("experimental") ? new Response("no", { status: 404 }) : Response.json({ data: { text: "Old." } })))
    expect(await (await post()).json()).toEqual({ data: { text: "Old." } })
    expect(seen.map((s) => s.path)).toEqual(["/api/experimental/generate", "/api/generate"])
  })

  test("a slow upstream ends at the client's deadline with an untyped-for-fallback error", async () => {
    serveOpenCode(async () => {
      await Bun.sleep(1000)
      return Response.json({ data: { text: "late" } })
    })
    const started = Date.now()
    const reply = await post({ "X-Hex-Deadline-Ms": "100" })
    expect(reply.status).toBe(504)
    expect((await reply.json())._tag).toBe("CompanionError")
    expect(Date.now() - started).toBeLessThan(900)
  })

  test("no running service is an honest error, and loose or remote registrations are ignored", async () => {
    expect((await post()).status).toBe(503)
    serveOpenCode(() => Response.json({ data: { text: "x" } }))
    chmodSync(join(config.stateDirectory, "service.json"), 0o644)
    expect((await post()).status).toBe(503)
    chmodSync(join(config.stateDirectory, "service.json"), 0o600)
    writeFileSync(join(config.stateDirectory, "service.json"), JSON.stringify({ url: "http://evil.example", pid: 1 }))
    chmodSync(join(config.stateDirectory, "service.json"), 0o600)
    expect((await post()).status).toBe(503)
  })
})
