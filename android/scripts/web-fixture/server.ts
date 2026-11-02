// The local test page for Chrome on the emulator. Each field reports its content here, so a
// test can read what Hex inserted without an accessibility snapshot, which would suppress Hex.
const values: Record<string, string> = {}
const page = `<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<title>Web fields</title><style>body{font:18px sans-serif;padding:12px}textarea,input,[contenteditable]{display:block;width:92%;font-size:18px;margin:12px 0;padding:8px;border:1px solid #888}textarea{height:110px}</style>
<h3>Web fields</h3>
<textarea id=area placeholder="Textarea"></textarea>
<input id=single placeholder="Single line">
<input id=secret type=password placeholder="Password">
<input id=card autocomplete=cc-number placeholder="Card number">
<input id=phone type=tel placeholder="Phone">
<div id=rich contenteditable=true><b>Rich</b> text</div>
<script>
for (const el of document.querySelectorAll('textarea,input,[contenteditable]')) {
  const send = () => fetch('/report', {method: 'POST', body: JSON.stringify({id: el.id, value: el.value ?? el.innerHTML})})
  el.addEventListener('input', send)
}
</script>`
Bun.serve({
  hostname: "127.0.0.1",
  port: Number(process.env.PORT ?? 8766),
  async fetch(request) {
    const { pathname } = new URL(request.url)
    if (pathname === "/report" && request.method === "POST") {
      const { id, value } = await request.json()
      values[id] = value
      return new Response("ok")
    }
    if (pathname === "/values") return Response.json(values)
    return new Response(page, { headers: { "Content-Type": "text/html" } })
  },
})
