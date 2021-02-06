# smol-quiche

This is a simple test project to see how well smol and quiche interacts.

If this was successful, I would have used this to build a DNS Stub resolver that backends to Google/Cloudflare's DNS over HTTPS service, using Quic.

Unfortunately, I found that I was getting over 100ms in round trip for a simple HTTP GET request. This could be a problem with the google infrastructure, the anycast routing, or any number of issues.

For now I will shelve this project until I have more time.