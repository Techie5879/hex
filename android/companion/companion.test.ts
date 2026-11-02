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
  config = { directory: join(root, "data") }
  token = addDevice(config, "phone")
})
afterEach(() => rmSync(root, { recursive: true, force: true }))

describe("authentication", () => {
  test("every route refuses a missing or wrong token", async () => {
    for (const path of ["/v1/profile", "/v1/health", "/nope"]) {
      expect((await call(path, { auth: null })).status).toBe(401)
      expect((await call(path, { auth: "wrong" })).status).toBe(401)
    }
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
