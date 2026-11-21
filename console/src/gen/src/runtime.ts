// A value import back into `models.ts`, which imports `JsonValue` from
// this file in the other direction. The cycle is fine and already
// established by `rpc-runtime.ts.j2`'s identical import: the other leg is
// `import type`, which is erased entirely at compile time.
import { encodeBinaryAsJson } from "./models.js";
import { COMPACT_JSON_CONTENT_TYPE, RestCompactSession } from "./compact.js";
import type { RestCompactCall } from "./compact.js";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

// Hex-encoded SHA-256 of the schema's source bytes (issue #178) — baked in
// at generation time from `TypeScriptGeneratorConfig::schema_sha256`. Sent
// as `x-cratestack-schema-sha` on every request so a client compiled
// against a stale `.cstack` schema shows up as a server-side
// `tracing::warn!`, never a rejection. Empty when the CLI wasn't given a
// schema fingerprint (e.g. this crate used as a library directly, or a
// test) — the header is simply omitted in that case.
export const SCHEMA_SHA256: string = "17211bfe5d841173df5df7ea8485da0d7a932097b1a100302342b97b298c6fee";
const SCHEMA_SHA_HEADER = "x-cratestack-schema-sha";

// Per-op contract digests (binding version 2, cratestack#1123): what a signed
// request to each op binds into its COSE AAD, keyed like the Rust client's
// `OP_CONTRACTS` (the RPC op id, or "<METHOD> <route template>" on REST, plus
// `batch` for `transport rpc`). Computed by the same `cratestack_core`
// function the Rust macros call, so the two cannot disagree. Not used by the
// unsigned runtime below: the sealers (ADR 0006 §11) read them.
export const OP_CONTRACTS: Readonly<Record<string, string>> = {
  "DELETE /comments/{id}": "d6c933fb1955fa4fc8867f347d04fb0e76d39ab44c8b6a428343aa758ea96f00",
  "DELETE /events/{id}": "9d7c301cc90b1cda9c22f9ec28ba30e46a7afc0603c26ce4fca31b7dbef4a301",
  "DELETE /items/{id}": "d0687a66f2bf0920a9360170586421b6d327c3d09a3d9f14933c756618b87df0",
  "DELETE /principals/{id}": "670cd82b1afedc446b46cf5a0e7acf9850f0c079c378462bac3900cbf26b9b79",
  "DELETE /projects/{id}": "78e2e7ebff80e268ea4c0cd048c082365187de9aac9e9f6c58a61df7d79dfc92",
  "DELETE /task_deps/{id}": "d7f6027395ef44748f93fab9bc84313d87798bc0bbf4aa8662b82f58afe87b0a",
  "DELETE /tasks/{id}": "989541e593260d8da18f3c080b49e792a2a4477e47ed0a93c7a6b28193ed3ec4",
  "DELETE /team_members/{id}": "fa1e69dde5de6ff77ebc7354b2351796818d04c44c72444892de802d6958303a",
  "DELETE /teams/{id}": "d65cdbe23357f5334a0aca59e81cf77daa5faaee27b3f280eb371c28d0da32a0",
  "DELETE /wake_targets/{id}": "7a48ae2c8530645c41ceda060e6573b6f5dca9910711db6d70b0abee7a667763",
  "GET /comments": "252704f5fd20273bbe2a51813290b7d815a2942d3d720e3fdd2777b37b112b20",
  "GET /comments/{id}": "812a37c5fcb1e908c0e10d9fea472e16eb245ae4440806a85af60c7458bf241a",
  "GET /events": "71bbd8e11021490d87d959c069f1a3aac613219cd4fa39b6cedd1c64529cc743",
  "GET /events/{id}": "eedafc68fd4d6339745d68e2a2ec6c2c094f186551b806da86867bda72ac9375",
  "GET /items": "71ba3a3d1bdfbe8d0b75e32dfbd81ca8d924191eabcdf3ad7449429d24765077",
  "GET /items/{id}": "7c33b9fabdc5f75e4c5f75b83ece2e7ee84c238bf1b423f575b4f94488b7e673",
  "GET /principals": "3180a1aea2bfc8eedf6419477f3ce894fcabd6321adc87bcebfd39352f11874f",
  "GET /principals/{id}": "a6a4076beadabd709b069af71d71e56aeed4ca6bd26da4102bb39abc5765ad8e",
  "GET /projects": "bd5dfa48af4992535447a38baeb8ca690fac694009ff27a108a160b14ea45e48",
  "GET /projects/{id}": "c7ebb1d1efa5fe0f572f29632b563fe9271b5dffef378527063372cf02dc6d87",
  "GET /task_deps": "48f351113974795978f028f88e8561fcf0580afc24dbe6c9e9e1470ef26a0f9d",
  "GET /task_deps/{id}": "87f336970b2cf59fd31c537f873c18e6698165aab083666a932dd588432014f8",
  "GET /tasks": "e5ea20cd8c20a6f764144e124700d718dafb200502542be4bb114ac8a18a1d71",
  "GET /tasks/{id}": "ba9e4686e6cd7204151523e3856ab30da00d3e2e0d388e654782c66846fc4ac8",
  "GET /team_members": "0cf3f7dfe018331f9d761e445107382cce268c082b82985403ded4e37e06ccd8",
  "GET /team_members/{id}": "7570156114c3552e245f886a9a7241c6c7bc146290bd8d1f25a2c214830cdadc",
  "GET /teams": "fdb441404359923eb7205695e8fe7137d5624e7c0c31e496fdb6f6b34bd6129c",
  "GET /teams/{id}": "433aa8c74f7661afbc8909ed28b858f57b93c049ebf26b2a4976b5c7e963446c",
  "GET /wake_targets": "cfc638269f73ac0c29cd1f81b7044f4466f90bcf398da35c7ec0b017a97faa3d",
  "GET /wake_targets/{id}": "50ea4a5d2480583d22a8de95ec53ee556d356ef34b4316016a2ea0c135fd9fd6",
  "PATCH /comments/{id}": "7fbd566f299489795977f962756fd2498a8aa13916be743d6aab7cae6b06c4d8",
  "PATCH /events/{id}": "2df1cae3b695d4205264ac2c4e05e4c27c887c106c9bab617e5199ef22df608a",
  "PATCH /items/{id}": "4c0c10cdacc0864cffc29a11dc7764a6ec6e97408cc06b1820dfa82d60e459ae",
  "PATCH /principals/{id}": "c403995b6980e10855e3519e949e900916b2c2ecba8de561f4c00170da8bb59c",
  "PATCH /projects/{id}": "92fcb058aa4ded2518ed4f82cdb105c4839bea57ae3d489c63ed6bb65808f669",
  "PATCH /task_deps/{id}": "1cd44baae427c9227895372d086ab3f00dcd1bfa04dd87da6e31e1ba8648206e",
  "PATCH /tasks/{id}": "42d4bb4b6ec1f0482291b48a6c65108e751bc550c4c12d69afd42432a6b6567e",
  "PATCH /team_members/{id}": "7a310516a6b3c9045199eae016883f986409787816521e6614a002f82b034298",
  "PATCH /teams/{id}": "4eeb4e0b8a7df43580489fd20985eb5c8c7a5d597f682a77b185dd81ec090ed8",
  "PATCH /wake_targets/{id}": "d0827ca9dac5de60b0e12898d6a08338b888509d8970c8bde50f8dd07ba0c3ec",
  "POST /$procs/block": "a23f1d8eb7aab2e5939dab61f9e462a2b465a94cf70abd3cff025031e6be7003",
  "POST /$procs/cancelTask": "d26e17de2cbdee50cc3f482b9f0546a9fdc0028231555264d1575d1cc866882f",
  "POST /$procs/changesSince": "73ef6f6549750b075bbf3ab0e7af1c08e8250704764bc10a606edc2258d04f0a",
  "POST /$procs/checkItem": "58da046fcac3a521a2dd4f4370ea8f83220b43c30be6263c2073ee0fd682c73c",
  "POST /$procs/claimNext": "deaad396c9669624d178c6bf1b0d6c29d3b1f63d6185dfe6e5d40900cdd1f772",
  "POST /$procs/claimTask": "a6433f369a60860e20c02b94c5df6b91e39bdc1e2408646cf83654efb6403654",
  "POST /$procs/comment": "9bf3467d0c40f81dcf0ad31ee02b998f35ffd3d8c1b65df4f58e324d5a6de4c9",
  "POST /$procs/complete": "268c592811e983ac706ef16c6d400258b121a9b95d703b5fb359b60e360ad9e5",
  "POST /$procs/getBoard": "34ae1a5c868506a84c9278af32ff148a5332ba8929e4abee05401ab8accf4afc",
  "POST /$procs/getTaskMarkdown": "43ed309454be689d7596b680ba61c2a9e0fb591d7f6cb0d22b32e42fc2c318fc",
  "POST /$procs/heartbeat": "878d488f60dad3d197e11fb4657ee5693ed399e6939adbf884f19e4a42c87d5c",
  "POST /$procs/projectOverview": "431f983858238bfd12d9b994a4e8212e97e97fb14093890a645882f39e3e1ef3",
  "POST /$procs/projectTasks": "a2080d50becb7f7b772328d78731192a78bab7a3ef774456d18c4ac210e9322e",
  "POST /$procs/release": "07aa31258c947637e103bc5a92f76a4f36a7ee1ea4ef95e980394c935851586b",
  "POST /$procs/taskCreate": "37cfbaa956eeb3bac878a1febdb334379f6876049b44e66ba31a124311fcabee",
  "POST /$procs/taskDetail": "a5c86b847e2f0106e05e1c8f9547978117df3c2f3a59f363e5cdbbbb01039863",
  "POST /$procs/taskEdit": "ba77ee8a471f05ad86ec8281e5665239aceb7d670d5e8d6df023eb55832e36fb",
  "POST /$procs/unblockTask": "b4f59321ee45bb14473da12a7bae596559caf4d8af95bf27f703072c5ec4faaf",
  "POST /$procs/waitForWork": "6e402f109599d77aa3289b4b6d2a6f1cf5ef6b61f9c8c2413522c9e8a7adf749",
  "POST /$procs/whoami": "0dec7266678fcb69728eb3a3ec558adfe40185a41a33a62485d2fa0a697f6324",
  "POST /comments": "d118136e971b877d0cde8cb7e5c569a1a59fa3eb6c518d1d77cfa8f24efa609f",
  "POST /events": "8a6c6519daf5684957b878e411b7c611992c1a8b9766e542c8fc3805d62ca5b7",
  "POST /items": "a49b659e7b027fce96116d06dc192904fcc28f48a046b24395008a08f171a490",
  "POST /principals": "6d4f0a7d7c5effd3cb71d4d795e8b1e0925b6fde92a0ad248552b441004ddaa2",
  "POST /projects": "89b43b647216334e1c468446113bdcb326b0ca4135c66fb03be9b415df0091f3",
  "POST /task_deps": "33ec06c196a3b039a6ba26be1173957cb2551acea29a5fb64b9db751734b2146",
  "POST /tasks": "a605cbab2c2a476e0422fca2db0f0bed01182ea86c246b4d1e639e7d97621ee7",
  "POST /team_members": "17b72dea1a3fc3d8771dccb959e47d9fc36c00c6b80dcd3fca5d336d69804b09",
  "POST /teams": "b66c8e0cffe1e39bc53c8622d43b92692ffa69939bf95e4f5567be0b3e61d7ee",
  "POST /wake_targets": "6d8fd0b394dcb1d346a10da285d6d67850c7d2b49271883d27367f6b78d8d815",
};
// Hex of the whole-contract digest: moves when any op's contract does.
export const CLIENT_CONTRACT_SHA256: string = "6e0dc3a3836e4ee63efa401862565d60161098ae0161269fde1e8244fe2d04ae";

// Base64 of the compact wire key tables for every op above (the artifact
// `cratestack_core::compact::KeyTables::to_bytes` writes), read only when
// `compactKeys` is on (see `./compact`).
export const KEY_TABLES: string = "gwGYQ4xoYXV0aG9ySWQAZGJvZHkAaWNyZWF0ZWRBdABiaWQAZHRhc2sFZnRhc2tJZACOZGRvbmUAZmRvbmVCeQBiaWQAaHBvc2l0aW9uAGR0YXNrBWZ0YXNrSWQAZHRleHQAmBppYWdlbnRLaW5kAGljcmVhdGVkQXQAamRpc2FibGVkQXQAYmlkAGdpc0FkbWluAGRraW5kAGpsYXN0U2VlbkF0AGltYXhMZWFzZXMAa21lbWJlcnNoaXBzggAHZG5hbWUAaW9pZGNMb2dpbgBnb3duZXJJZABncnVudGltZQCQYmlkAGNrZXkAZG5hbWUAam5leHROdW1iZXIAZ3JlcG9VcmwAZXRhc2tzggAFZHRlYW0GZnRlYW1JZACYJGRib2R5AGljbGFpbWVkQnkAaGNvbW1lbnRzggABaWNyZWF0ZWRBdABpY3JlYXRlZEJ5AGJpZABlaXRlbXOCAAJqbGVhc2VFcG9jaABubGVhc2VFeHBpcmVzQXQAZm51bWJlcgBocGFyZW50SWQAaHByaW9yaXR5AGdwcm9qZWN0BGlwcm9qZWN0SWQAZnJlc3VsdABmc3RhdHVzAGV0aXRsZQBpdXBkYXRlZEF0AIhiaWQAZ21lbWJlcnOCAAdkbmFtZQBocHJvamVjdHOCAASMYmlkAGlwcmluY2lwYWwDa3ByaW5jaXBhbElkAGRyb2xlAGR0ZWFtBmZ0ZWFtSWQAjGhhdXRob3JJZABkYm9keQBpY3JlYXRlZEF0AGJpZABkdGFzaw1mdGFza0lkAJBnYWN0b3JJZABiYXQAZGtpbmQAZ3BheWxvYWQAZ3Byb2plY3QMaXByb2plY3RJZABjc2VxAGZ0YXNrSWQAjmRkb25lAGZkb25lQnkAYmlkAGhwb3NpdGlvbgBkdGFzaw1mdGFza0lkAGR0ZXh0AJgaaWFnZW50S2luZABpY3JlYXRlZEF0AGpkaXNhYmxlZEF0AGJpZABnaXNBZG1pbgBka2luZABqbGFzdFNlZW5BdABpbWF4TGVhc2VzAGttZW1iZXJzaGlwc4IAD2RuYW1lAGlvaWRjTG9naW4AZ293bmVySWQAZ3J1bnRpbWUAkGJpZABja2V5AGRuYW1lAGpuZXh0TnVtYmVyAGdyZXBvVXJsAGV0YXNrc4IADWR0ZWFtDmZ0ZWFtSWQAmCRkYm9keQBpY2xhaW1lZEJ5AGhjb21tZW50c4IACGljcmVhdGVkQXQAaWNyZWF0ZWRCeQBiaWQAZWl0ZW1zggAKamxlYXNlRXBvY2gAbmxlYXNlRXhwaXJlc0F0AGZudW1iZXIAaHBhcmVudElkAGhwcmlvcml0eQBncHJvamVjdAxpcHJvamVjdElkAGZyZXN1bHQAZnN0YXR1cwBldGl0bGUAaXVwZGF0ZWRBdACIYmlkAGdtZW1iZXJzggAPZG5hbWUAaHByb2plY3RzggAMjGJpZABpcHJpbmNpcGFsC2twcmluY2lwYWxJZABkcm9sZQBkdGVhbQ5mdGVhbUlkAIxoYXV0aG9ySWQAZGJvZHkAaWNyZWF0ZWRBdABiaWQAZHRhc2sUZnRhc2tJZACOZGRvbmUAZmRvbmVCeQBiaWQAaHBvc2l0aW9uAGR0YXNrFGZ0YXNrSWQAZHRleHQAmBppYWdlbnRLaW5kAGljcmVhdGVkQXQAamRpc2FibGVkQXQAYmlkAGdpc0FkbWluAGRraW5kAGpsYXN0U2VlbkF0AGltYXhMZWFzZXMAa21lbWJlcnNoaXBzggAXZG5hbWUAaW9pZGNMb2dpbgBnb3duZXJJZABncnVudGltZQCQYmlkAGNrZXkAZG5hbWUAam5leHROdW1iZXIAZ3JlcG9VcmwAZXRhc2tzggAUZHRlYW0WZnRlYW1JZACYJGRib2R5AGljbGFpbWVkQnkAaGNvbW1lbnRzggAQaWNyZWF0ZWRBdABpY3JlYXRlZEJ5AGJpZABlaXRlbXOCABFqbGVhc2VFcG9jaABubGVhc2VFeHBpcmVzQXQAZm51bWJlcgBocGFyZW50SWQAaHByaW9yaXR5AGdwcm9qZWN0E2lwcm9qZWN0SWQAZnJlc3VsdABmc3RhdHVzAGV0aXRsZQBpdXBkYXRlZEF0AIhpZGVwZW5kc09uAGJpZABkdGFzaxRmdGFza0lkAIhiaWQAZ21lbWJlcnOCABdkbmFtZQBocHJvamVjdHOCABOMYmlkAGlwcmluY2lwYWwSa3ByaW5jaXBhbElkAGRyb2xlAGR0ZWFtFmZ0ZWFtSWQAjGhhdXRob3JJZABkYm9keQBpY3JlYXRlZEF0AGJpZABkdGFzaxgcZnRhc2tJZACOZGRvbmUAZmRvbmVCeQBiaWQAaHBvc2l0aW9uAGR0YXNrGBxmdGFza0lkAGR0ZXh0AJgaaWFnZW50S2luZABpY3JlYXRlZEF0AGpkaXNhYmxlZEF0AGJpZABnaXNBZG1pbgBka2luZABqbGFzdFNlZW5BdABpbWF4TGVhc2VzAGttZW1iZXJzaGlwc4IAGB5kbmFtZQBpb2lkY0xvZ2luAGdvd25lcklkAGdydW50aW1lAJBiaWQAY2tleQBkbmFtZQBqbmV4dE51bWJlcgBncmVwb1VybABldGFza3OCABgcZHRlYW0YHWZ0ZWFtSWQAmCRkYm9keQBpY2xhaW1lZEJ5AGhjb21tZW50c4IAGBhpY3JlYXRlZEF0AGljcmVhdGVkQnkAYmlkAGVpdGVtc4IAGBlqbGVhc2VFcG9jaABubGVhc2VFeHBpcmVzQXQAZm51bWJlcgBocGFyZW50SWQAaHByaW9yaXR5AGdwcm9qZWN0GBtpcHJvamVjdElkAGZyZXN1bHQAZnN0YXR1cwBldGl0bGUAaXVwZGF0ZWRBdACIYmlkAGdtZW1iZXJzggAYHmRuYW1lAGhwcm9qZWN0c4IAGBuMYmlkAGlwcmluY2lwYWwYGmtwcmluY2lwYWxJZABkcm9sZQBkdGVhbRgdZnRlYW1JZACIZWFnZW50GBpnYWdlbnRJZABiaWQAZHR5cGUAmChkYm9keQBqY2xhaW1lZF9ieQBqY3JlYXRlZF9hdABqZGVwZW5kc19vbgBiaWQAZmxhYmVscwBrbGVhc2VfZXBvY2gAcGxlYXNlX2V4cGlyZXNfYXQAamxlYXNlX2xpdmUAZm51bWJlcgBqb3Blbl9pdGVtcwBmcGFyZW50AGhwcmlvcml0eQBncHJvamVjdABlcmVhZHkAZnJlc3VsdABmc3RhdHVzAGV0aXRsZQBrdG90YWxfaXRlbXMAanVwZGF0ZWRfYXQAhmJpZABlZXBvY2gAZnJlYXNvbgCCYmlkAIRmZXZlbnRzggAYJWtuZXh0X2N1cnNvcgCOZWFnZW50AGRkb25lAGVlcG9jaABocG9zaXRpb24AZnJlYXNvbgBmcmVzdWx0AGV0aXRsZQCOZWFjdG9yAGJhdABka2luZABncGF5bG9hZBgkZ3Byb2plY3QAY3NlcQBkdGFzawCGZmN1cnNvcgBncHJvamVjdABlbGltaXQAhmJhdABmYXV0aG9yAGRib2R5AJgwZGJvZHkAamNsYWltZWRfYnkAaGNvbW1lbnRzggAYJ2pjcmVhdGVkX2F0AGxkZXBlbmRlbmNpZXOCABgqamRlcGVuZHNfb24AYmlkAGVpdGVtc4IAGClmbGFiZWxzAGtsZWFzZV9lcG9jaABwbGVhc2VfZXhwaXJlc19hdABqbGVhc2VfbGl2ZQBmbnVtYmVyAGpvcGVuX2l0ZW1zAGZwYXJlbnQAaHByaW9yaXR5AGdwcm9qZWN0AGVyZWFkeQBmcmVzdWx0AGZzdGF0dXMAaHN1YnRhc2tzggAYKmV0aXRsZQBrdG90YWxfaXRlbXMAanVwZGF0ZWRfYXQAiGRkb25lAGdkb25lX2J5AGhwb3NpdGlvbgBkdGV4dACGYmlkAGZzdGF0dXMAZXRpdGxlAIhiaWQAZWVwb2NoAGhwb3NpdGlvbgBkZG9uZQCGZWxlYXNlGC1kbm9uZQBkdGFzaxgvhGVlcG9jaABqZXhwaXJlc19hdACGYmF0AGZhdXRob3IAZGJvZHkAmDBkYm9keQBqY2xhaW1lZF9ieQBoY29tbWVudHOCABguamNyZWF0ZWRfYXQAbGRlcGVuZGVuY2llc4IAGDFqZGVwZW5kc19vbgBiaWQAZWl0ZW1zggAYMGZsYWJlbHMAa2xlYXNlX2Vwb2NoAHBsZWFzZV9leHBpcmVzX2F0AGpsZWFzZV9saXZlAGZudW1iZXIAam9wZW5faXRlbXMAZnBhcmVudABocHJpb3JpdHkAZ3Byb2plY3QAZXJlYWR5AGZyZXN1bHQAZnN0YXR1cwBoc3VidGFza3OCABgxZXRpdGxlAGt0b3RhbF9pdGVtcwBqdXBkYXRlZF9hdACIZGRvbmUAZ2RvbmVfYnkAaHBvc2l0aW9uAGR0ZXh0AIZiaWQAZnN0YXR1cwBldGl0bGUAhGdwcm9qZWN0AGZsYWJlbHMAgmJvawCGYmlkAGRib2R5AGVlcG9jaACGYmlkAGVlcG9jaABmcmVzdWx0AIJobWFya2Rvd24Agmdwcm9qZWN0AIZlZXBvY2gAamV4cGlyZXNfYXQAYmlkAIZiaWQAZWVwb2NoAGR0dGxTAJRnYmxvY2tlZABnY2xhaW1lZABkZG9uZQBja2V5AGtsZWFzZV90dGxfcwBkbmFtZQBkb3BlbgBlcmVhZHkAaHJlcG9fdXJsAGR0ZWFtAIhncHJvamVjdABmc3RhdHVzAGVsYWJlbABpcmVhZHlPbmx5AJBncHJvamVjdABldGl0bGUAZGJvZHkAZWl0ZW1zAGZwYXJlbnQAaWRlcGVuZHNPbgBmbGFiZWxzAGhwcmlvcml0eQCKYmlkAGV0aXRsZQBkYm9keQBocHJpb3JpdHkAZmxhYmVscwCYKGRib2R5AGpjbGFpbWVkX2J5AGpjcmVhdGVkX2F0AGpkZXBlbmRzX29uAGJpZABmbGFiZWxzAGtsZWFzZV9lcG9jaABwbGVhc2VfZXhwaXJlc19hdABqbGVhc2VfbGl2ZQBmbnVtYmVyAGpvcGVuX2l0ZW1zAGZwYXJlbnQAaHByaW9yaXR5AGdwcm9qZWN0AGVyZWFkeQBmcmVzdWx0AGZzdGF0dXMAZXRpdGxlAGt0b3RhbF9pdGVtcwBqdXBkYXRlZF9hdACCZXJlYWR5ggAYPoRncHJvamVjdABodGltZW91dFMAmChkYm9keQBqY2xhaW1lZF9ieQBqY3JlYXRlZF9hdABqZGVwZW5kc19vbgBiaWQAZmxhYmVscwBrbGVhc2VfZXBvY2gAcGxlYXNlX2V4cGlyZXNfYXQAamxlYXNlX2xpdmUAZm51bWJlcgBqb3Blbl9pdGVtcwBmcGFyZW50AGhwcmlvcml0eQBncHJvamVjdABlcmVhZHkAZnJlc3VsdABmc3RhdHVzAGV0aXRsZQBrdG90YWxfaXRlbXMAanVwZGF0ZWRfYXQAhGRuYW1lAGRyb2xlAJJqYWdlbnRfa2luZABuY3VycmVudF9sZWFzZXOCABhBaGlzX2FkbWluAGRraW5kAGptYXhfbGVhc2VzAGRuYW1lAGhwcm9qZWN0cwBncnVudGltZQBldGVhbXOCABhCmEaEdURFTEVURSAvY29tbWVudHMve2lkfVgg1skz+xlV+k/Ihn80fQT7DnbTmrRMi2pCg0OqdY6pbwAAAYRzREVMRVRFIC9ldmVudHMve2lkfVggnXwwHMkLHNqcIvnsKLow5Gp6/AYDwmzk/KMbfb70owEACYRyREVMRVRFIC9pdGVtcy97aWR9WCDQaHpm8r8JIKk2AXBYZCG20yfD0Jo9nxSTPHVmGLh98AAChHdERUxFVEUgL3ByaW5jaXBhbHMve2lkfVggZwzYKxr+3ERrRs9aDnrP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";

export interface CratestackClientOptions {
  basePath?: string;
  fetch?: typeof fetch;
  headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);
  /** Opt in to compact wire keys: schema-declared fields travel as integer
   *  keys (`application/vnd.cratestack.compact+json`). Default `false`, which
   *  leaves the wire byte-identical to a client without the option. The
   *  request body goes compact only after the op has answered compact once,
   *  so an older server is never sent a body it cannot read. Loads the
   *  optional peer `@cratestack/cbor` on first use; without it, or with a
   *  release that predates `createCompactKeys`, the option is silently
   *  inert. Error bodies stay named. */
  compactKeys?: boolean;
}

/** Serializable request prepared before its first network attempt. */
export interface PreparedCall {
  version: 1;
  method: string;
  path: string;
  url: string;
  op: string;
  contract: string;
  keyTables: string;
  headers: [string, string][];
  body: string | null;
}

export interface CratestackRequestOptions {
  body?: unknown;
  headers?: HeadersInit | undefined;
  query?: Record<string, unknown> | undefined;
  signal?: AbortSignal | undefined;
}

// Issue #610: `request()` decodes the body and discards the `Response`,
// so no caller can ever reach a response header (`ETag`, most notably —
// the generated server stamps it on every `@version` model's GET/detail
// response, and requires it back as `If-Match` on PATCH/DELETE). This
// envelope is what `requestWithResponse()`/`getWithResponse()` return
// instead, so the header becomes reachable without changing `request()`'s
// existing return shape for every other call site.
export interface CratestackResponseEnvelope<T> {
  value: T;
  response: Response;
}

const RESERVED_ERROR_MEMBERS: ReadonlySet<string> = new Set(["code", "message", "details"]);

/** The members of a decoded error body beyond `code`, `message` and
 *  `details`: what an `AuthProvider` refusal adds (cratestack#1208), such
 *  as `{ error: "attestation_expired" }`. Every member is kept, as decoded;
 *  empty when `body` is not an object. */
function extraErrorMembers(body: unknown): Readonly<Record<string, unknown>> {
  const extra: Record<string, unknown> = {};
  if (typeof body === "object" && body !== null && !Array.isArray(body)) {
    for (const [name, value] of Object.entries(body)) {
      if (!RESERVED_ERROR_MEMBERS.has(name)) {
        extra[name] = value;
      }
    }
  }
  return Object.freeze(extra);
}

/** The answer's `WWW-Authenticate` value, raw and unparsed (cratestack#1243);
 *  empty when there is none. `Headers` joins two challenge lines with `, `,
 *  so they arrive as one entry holding both. */
function challengesOf(headers: Headers | undefined): readonly string[] {
  const value = headers?.get("www-authenticate");
  return Object.freeze(value ? [value] : []);
}

/** The extra member `error` of a step-up refusal (RFC 9470, cratestack#1244). */
export const INSUFFICIENT_USER_AUTHENTICATION = "insufficient_user_authentication";

/** What a step-up refusal asks for (cratestack#1244): an operation that
 *  requires a fresher or stronger authentication than the caller presented.
 *  Each requirement is present only when the refused operation declares it. */
export interface StepUpRequired {
  /** The longest time, in seconds, since the caller last authenticated. */
  readonly maxAge: number | undefined;
  /** The authentication context classes the operation accepts. */
  readonly acrValues: readonly string[];
  /** The authentication methods the operation requires. */
  readonly amr: readonly string[];
  /** Which requirements the caller failed: a subset of `"max_age"`, `"acr"`
   *  and `"amr"`. */
  readonly unmet: readonly string[];
}

/** Reads a step-up refusal once the transport's own test (`refused`: a 401,
 *  or `unauthenticated`) passed: `undefined` unless the extra member `error`
 *  is `insufficient_user_authentication`. Never throws: a missing or
 *  mistyped field reads as absent or empty. */
function readStepUp(
  refused: boolean,
  details: unknown,
  extra: Readonly<Record<string, unknown>>,
): StepUpRequired | undefined {
  if (!refused || extra["error"] !== INSUFFICIENT_USER_AUTHENTICATION) {
    return undefined;
  }
  const map: Record<string, unknown> =
    typeof details === "object" && details !== null && !Array.isArray(details)
      ? (details as Record<string, unknown>)
      : {};
  const strings = (name: string): readonly string[] => {
    const value = map[name];
    return Object.freeze(
      Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [],
    );
  };
  const raw = map["max_age"];
  const maxAge = typeof raw === "bigint" ? Number(raw) : raw;
  return Object.freeze({
    maxAge:
      typeof maxAge === "number" && Number.isSafeInteger(maxAge) && maxAge >= 0
        ? maxAge
        : undefined,
    acrValues: strings("acr_values"),
    amr: strings("amr"),
    unmet: strings("unmet"),
  });
}

/** Runs `call`; when it fails with a step-up refusal, hands the refusal to
 *  `reprove` and, when that resolves `true` (the user authenticated again),
 *  runs `call` **once** more and returns what it gives, even a second
 *  refusal. Never loops (cratestack#1244).
 *
 *  `reprove` resolves `false` when the user declined or the re-proof failed;
 *  the first refusal is then rethrown. Any other error, and one `reprove`
 *  itself throws, propagates as it is.
 *
 *  ```ts
 *  const receipt = await retryAfterStepUp(
 *    () => client.procedures.transfer(args),
 *    (stepUp) => session.reauthenticate({ maxAge: stepUp.maxAge }),
 *  );
 *  ``` */
export async function retryAfterStepUp<T>(
  call: () => Promise<T>,
  reprove: (stepUp: StepUpRequired) => Promise<boolean>,
): Promise<T> {
  try {
    return await call();
  } catch (error) {
    const stepUp = stepUpOf(error);
    if (stepUp === undefined || !(await reprove(stepUp))) {
      throw error;
    }
  }
  return call();
}

/** The error envelope a CrateStack server answers with. Mirrors
 *  `cratestack_core::CratestackErrorResponse`; the members an
 *  `AuthProvider` refusal adds beside these three are on
 *  {@link CratestackHttpError.extra}. */
export interface CratestackErrorResponse {
  code: string;
  message: string;
  details?: unknown;
}

function isErrorResponse(payload: unknown): payload is CratestackErrorResponse {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { code?: unknown }).code === "string"
  );
}

/** Thrown when the server answers with a non-2xx status. `payload` is the
 *  whole decoded body; the envelope's members are typed beside it: `body`
 *  (`code`, `message`, `details`), `extra` (every other member, as an
 *  `AuthProvider` refusal adds them) and `challenges` (the
 *  `WWW-Authenticate` value). */
export class CratestackHttpError extends Error {
  readonly status: number;
  readonly response: Response;
  readonly payload: unknown;
  /** The decoded envelope, or `undefined` when the payload is not one. */
  readonly body: CratestackErrorResponse | undefined;
  /** The envelope's `code` (`UNAUTHORIZED`, `FORBIDDEN`, ...). */
  readonly code: string | undefined;
  readonly details: unknown;
  readonly extra: Readonly<Record<string, unknown>>;
  readonly challenges: readonly string[];
  /** The step-up refusal this answer is (cratestack#1244): a 401 whose
   *  extra member `error` is `insufficient_user_authentication`, read from
   *  `details`; `undefined` for any other error. */
  readonly stepUp: StepUpRequired | undefined;

  constructor(response: Response, payload: unknown) {
    super(`CrateStack request failed with status ${response.status}`);
    this.name = "CratestackHttpError";
    this.status = response.status;
    this.response = response;
    this.payload = payload;
    this.body = isErrorResponse(payload) ? payload : undefined;
    this.code = this.body?.code;
    this.details = this.body?.details;
    this.extra = extraErrorMembers(this.body);
    this.challenges = challengesOf(response.headers);
    this.stepUp = readStepUp(this.status === 401, this.details, this.extra);
  }
}

/** The step-up refusal of a {@link CratestackHttpError} (its `stepUp`);
 *  `undefined` for anything else (cratestack#1244). */
export function stepUpOf(error: unknown): StepUpRequired | undefined {
  return error instanceof CratestackHttpError ? error.stepUp : undefined;
}

export class CratestackRuntime {
  readonly origin: string;
  readonly basePath: string;
  readonly fetchFn: typeof fetch;
  readonly defaultHeaders: HeadersInit | (() => HeadersInit | Promise<HeadersInit>) | undefined;
  private readonly compact: RestCompactSession | undefined;

  constructor(origin: string, options: CratestackClientOptions = {}) {
    this.origin = trimTrailingSlashes(origin);
    this.basePath = options.basePath ?? "/api/v2";
    // `.bind(globalThis)`, not the bare global — some browsers' `fetch`
    // is spec'd to throw `TypeError: Illegal invocation` when called
    // with a receiver other than the global object (verified for real:
    // storing the bare function on `this` and calling it as
    // `this.fetchFn(...)` reproduces exactly that in Chrome/Vite dev,
    // even though the same code runs fine under Node's `fetch`, which
    // is why this was never caught by a Node-only test). A caller-
    // supplied `options.fetch` is trusted to already be correctly bound.
    this.fetchFn = options.fetch ?? fetch.bind(globalThis);
    this.defaultHeaders = options.headers;
    this.compact =
      options.compactKeys === true
        ? new RestCompactSession(KEY_TABLES, Object.keys(OP_CONTRACTS))
        : undefined;
  }

  /** Prepare a keyed request for caller-owned durable storage before sending. */
  async prepare(method: string, path: string, options: CratestackRequestOptions = {}): Promise<PreparedCall> {
    if (!["POST", "PUT", "PATCH", "DELETE"].includes(method.toUpperCase())) throw new Error("prepare requires a mutation");
    const target = new URL(this.url(path, options.query));
    if (target.username || target.password) throw new Error("prepared URL must not contain userinfo");
    const headers = new Headers(await resolveHeaders(this.defaultHeaders));
    for (const [key, value] of new Headers(options.headers)) headers.set(key, value);
    if (!headers.get("Idempotency-Key")) throw new Error("prepare requires an Idempotency-Key");
    const routes = this.compact ?? new RestCompactSession(KEY_TABLES, Object.keys(OP_CONTRACTS));
    const op = routes.opOf(method, path);
    const contract = op === undefined ? undefined : OP_CONTRACTS[op];
    if (op === undefined || contract === undefined) throw new Error("unknown prepared route");
    let body = options.body === undefined ? null : JSON.stringify(encodeBinaryAsJson(options.body));
    headers.set("Accept", "application/json");
    if (body !== null) headers.set("Content-Type", "application/json");
    const compact = await this.compact?.begin(method, path, headers.get("Idempotency-Key") ?? undefined);
    if (compact !== undefined) {
      compact.applyHeaders(headers, body !== null);
      if (body !== null) body = compact.encodeBody(body);
    }
    if (body !== null && new TextEncoder().encode(body).length > 8 * 1024 * 1024) throw new Error("prepared body too large");
    for (const name of [...headers.keys()]) if (!["idempotency-key", "if-match", "accept", "content-type", "cratestack-contract"].includes(name)) headers.delete(name);
    const prepared: PreparedCall = { version: 1, method: method.toUpperCase(), path, url: target.toString(), op, contract, keyTables: KEY_TABLES, headers: [...headers], body };
    if (new TextEncoder().encode(JSON.stringify(prepared)).length > 16 * 1024 * 1024) throw new Error("prepared call too large");
    return prepared;
  }

  async executePrepared<T>(prepared: PreparedCall, signal?: AbortSignal): Promise<T> {
    const serialized = JSON.stringify(prepared);
    if (new TextEncoder().encode(serialized).length > 16 * 1024 * 1024) throw new Error("prepared call too large");
    prepared = JSON.parse(serialized) as PreparedCall;
    if (Object.keys(prepared).sort().join(",") !== "body,contract,headers,keyTables,method,op,path,url,version" || prepared.version !== 1 ||
      typeof prepared.method !== "string" || typeof prepared.path !== "string" || typeof prepared.url !== "string" ||
      typeof prepared.op !== "string" || typeof prepared.contract !== "string" || prepared.contract !== OP_CONTRACTS[prepared.op] || prepared.keyTables !== KEY_TABLES ||
      !Array.isArray(prepared.headers) || (prepared.body !== null && typeof prepared.body !== "string")) {
      throw new Error("invalid or incompatible prepared call");
    }
    const target = new URL(prepared.url);
    if (target.username || target.password) throw new Error("prepared URL must not contain userinfo");
    if (prepared.body !== null) JSON.parse(prepared.body);
    const names = new Set<string>();
    for (const pair of prepared.headers) {
      if (!Array.isArray(pair) || pair.length !== 2 || typeof pair[0] !== "string" || typeof pair[1] !== "string") throw new Error("invalid prepared header");
      const name = pair[0].toLowerCase();
      if (names.has(name) || !["idempotency-key", "if-match", "accept", "content-type", "cratestack-contract"].includes(name) || new TextEncoder().encode(pair[1]).length > 8192) throw new Error("invalid prepared header");
      names.add(name);
    }
    const routes = this.compact ?? new RestCompactSession(KEY_TABLES, Object.keys(OP_CONTRACTS));
    if (!["POST", "PUT", "PATCH", "DELETE"].includes(prepared.method) || routes.opOf(prepared.method, prepared.path) !== prepared.op ||
      this.url(prepared.path).split("?")[0] !== prepared.url.split("?")[0] ||
      (prepared.body !== null && new TextEncoder().encode(prepared.body).length > 8 * 1024 * 1024)) throw new Error("invalid prepared route or body");
    const saved = new Headers(prepared.headers);
    if (!saved.get("Idempotency-Key")) throw new Error("invalid prepared key");
    const headers = new Headers(await resolveHeaders(this.defaultHeaders));
    if (SCHEMA_SHA256 !== "") headers.set(SCHEMA_SHA_HEADER, SCHEMA_SHA256);
    for (const name of ["idempotency-key", "if-match", "accept", "content-type", "cratestack-contract"]) headers.delete(name);
    for (const [key, value] of saved) headers.set(key, value);
    const compact = await this.compact?.begin(prepared.method, prepared.path, headers.get("Idempotency-Key") ?? undefined);
    if (headers.has("Cratestack-Contract") && compact === undefined) throw new Error("compact decoder unavailable");
    const expected = new Headers();
    if (compact !== undefined) compact.applyHeaders(expected, prepared.body !== null);
    const contentType = saved.get("content-type");
    if (saved.get("accept") !== (compact ? expected.get("accept") : "application/json") ||
        saved.get("cratestack-contract") !== expected.get("cratestack-contract") ||
        (prepared.body !== null && contentType !== "application/json" && contentType !== "application/vnd.cratestack.compact+json") ||
        (contentType === "application/vnd.cratestack.compact+json" && compact === undefined)) throw new Error("invalid prepared media or selector");

    const response = await this.fetchFn(prepared.url, { method: prepared.method, headers, body: prepared.body, signal: signal ?? null });
    const payload = await readResponsePayload(response, compact);
    if (!response.ok) throw new CratestackHttpError(response, payload);
    return payload as T;
  }

  async request<T>(
    method: string,
    path: string,
    options: CratestackRequestOptions = {},
  ): Promise<T> {
    const { value } = await this.requestWithResponse<T>(method, path, options);
    return value;
  }

  // Same request as `request()`, but returns the `Response` alongside the
  // decoded value (issue #610) instead of discarding it — `request()` is
  // now a thin wrapper around this that keeps only `.value`, so every
  // existing call site's return shape is unchanged.
  async requestWithResponse<T>(
    method: string,
    path: string,
    options: CratestackRequestOptions = {},
  ): Promise<CratestackResponseEnvelope<T>> {
    const headers = new Headers(await resolveHeaders(this.defaultHeaders));
    if (SCHEMA_SHA256 !== "") {
      headers.set(SCHEMA_SHA_HEADER, SCHEMA_SHA256);
    }
    headers.set("Accept", "application/json");

    const effectiveHeaders = new Headers(headers);
    for (const [key, value] of new Headers(options.headers)) effectiveHeaders.set(key, value);
    const compact = await this.compact?.begin(method, path, effectiveHeaders.get("Idempotency-Key") ?? undefined);
    let body: BodyInit | undefined;
    if (options.body !== undefined) {
      headers.set("Content-Type", "application/json");
      // `encodeBinaryAsJson` rewrites every `Uint8Array` (a `Bytes`
      // field's client-side type) into the integer array the wire uses —
      // `JSON.stringify` alone turns one into an index-keyed object no
      // server-side `Vec<u8>` can decode — and every `bigint` (a `BigInt`
      // field) into its canonical decimal string, which `JSON.stringify`
      // alone refuses to serialize at all. Unconditional here, unlike the
      // RPC transport's codec-dependent handling, because a REST request
      // body is always JSON.
      body = JSON.stringify(encodeBinaryAsJson(options.body));
    }
    if (compact !== undefined) {
      compact.applyHeaders(headers, body !== undefined);
      if (typeof body === "string") {
        body = compact.encodeBody(body);
      }
    }

    for (const [key, value] of new Headers(options.headers)) {
      headers.set(key, value);
    }

    const response = await this.fetchFn(this.url(path, options.query), {
      method,
      headers,
      body: body ?? null,
      signal: options.signal ?? null,
    });

    if (!response.ok) {
      // A `415` sends the op back to named bodies (see `./compact`).
      compact?.noteFailure(response.status);
    }
    const payload = await readResponsePayload(response, compact);
    if (!response.ok) {
      throw new CratestackHttpError(response, payload);
    }
    return { value: payload as T, response };
  }

  get<T>(path: string, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("GET", path, options);
  }

  // Issue #610: the READ half of the ETag/If-Match round trip — read
  // `.response.headers.get("etag")` off the result, then pass that value
  // as `ifMatch` to a generated model's `update`/`delete` method.
  getWithResponse<T>(
    path: string,
    options: Omit<CratestackRequestOptions, "body"> = {},
  ): Promise<CratestackResponseEnvelope<T>> {
    return this.requestWithResponse<T>("GET", path, options);
  }

  post<T>(path: string, body: unknown, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("POST", path, { ...options, body });
  }

  patch<T>(path: string, body: unknown, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("PATCH", path, { ...options, body });
  }

  delete<T>(path: string, options: Omit<CratestackRequestOptions, "body"> = {}): Promise<T> {
    return this.request<T>("DELETE", path, options);
  }

  private url(path: string, query?: Record<string, unknown>): string {
    const normalizedBase = trimTrailingSlashes(this.basePath);
    const normalizedPath = path.startsWith("/") ? path : `/${path}`;
    const url = new URL(`${normalizedBase}${normalizedPath}`, `${this.origin}/`);
    appendQuery(url.searchParams, query);
    return url.toString();
  }
}

async function resolveHeaders(
  headers: HeadersInit | (() => HeadersInit | Promise<HeadersInit>) | undefined,
): Promise<HeadersInit | undefined> {
  if (typeof headers === "function") {
    return headers();
  }
  return headers;
}

async function readResponsePayload(
  response: Response,
  compact?: RestCompactCall,
): Promise<unknown> {
  if (response.status === 204) {
    return undefined;
  }

  const text = await response.text();
  if (text.length === 0) {
    return undefined;
  }

  const contentType = response.headers.get("Content-Type") ?? "";
  // A compact body is transcoded back to plain JSON; an error body never is.
  if (
    compact !== undefined &&
    response.ok &&
    contentType.split(";", 1)[0]?.trim() === COMPACT_JSON_CONTENT_TYPE
  ) {
    return JSON.parse(compact.decodeResponse(text));
  }
  if (contentType.includes("application/json")) {
    return JSON.parse(text);
  }
  return text;
}

function appendQuery(searchParams: URLSearchParams, query?: Record<string, unknown>): void {
  if (!query) {
    return;
  }

  for (const [key, value] of Object.entries(query)) {
    appendQueryValue(searchParams, key, value);
  }
}

function appendQueryValue(searchParams: URLSearchParams, key: string, value: unknown): void {
  if (value === undefined || value === null) {
    return;
  }

  if (Array.isArray(value)) {
    for (const item of value) {
      appendQueryValue(searchParams, key, item);
    }
    return;
  }

  if (typeof value === "object") {
    // The same walk the body gets: an object-valued query entry
    // (`computedParams`) may hold a `bigint`, which `JSON.stringify` throws
    // on. A scalar `bigint` below needs nothing: `String(7n)` is `"7"`.
    searchParams.set(key, JSON.stringify(encodeBinaryAsJson(value)));
    return;
  }

  searchParams.append(key, String(value));
}

function trimTrailingSlashes(value: string): string {
  let end = value.length;
  while (end > 0 && value.charCodeAt(end - 1) === 47) {
    end--;
  }
  return value.slice(0, end);
}