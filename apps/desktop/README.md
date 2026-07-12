# Void — React Frontend

**React 19** + **Vite 7** + **TypeScript** frontend with **TailwindCSS v4**. Follows a strict architecture: dumb components, business logic in contexts, state changes in hooks.

## Architecture

```mermaid
graph TB
    COMPONENTS["<b>Components (Dumb UI)</b><br/>auth · channel · chat · layout<br/>settings · sidebar · stream · ui"]
    CONTEXTS["<b>Contexts (Business Logic)</b><br/>Auth · Voice · Stream · Chat<br/>Dm · Server · Toast · Bento"]
    HOOKS["<b>Hooks (State Logic)</b><br/>useDashboardState · useBentoLayout<br/>useNetworkStats · usePushToTalk<br/>useVoiceActivity · useProfileSettings"]
    API["<b>API Layer</b><br/>http-client · auth.api<br/>friends.ws · signalingBus"]
    WORKERS["<b>Workers (Off main thread)</b><br/>noise-gate.worklet · analyzer.worker"]

    COMPONENTS --> CONTEXTS --> HOOKS
    HOOKS --> API
    HOOKS --> WORKERS
```

*Each box lists its sub-modules — see [File Structure](#file-structure) below for the full breakdown.*

## File Structure

```
src/
├── api/                  # HTTP client + endpoint modules
│   ├── http-client.ts    # Protobuf/JSON content negotiation (REST)
│   ├── auth.api.ts       # Auth REST endpoints
│   └── friends.ws.ts     # Friends WS-RPC client (canonical)
├── components/           # Dumb, agnostic UI components
│   ├── auth/             # Login screen
│   ├── channel/          # Channel list, items, creation modal
│   ├── chat/             # Chat panel
│   ├── layout/           # Main layout, sidebar, title bar
│   ├── settings/         # Settings panels (voice, profile, etc.)
│   ├── sidebar/          # Server sidebar, user footer, members
│   ├── stream/           # Stream cards, audio renderer
│   └── ui/               # Shared UI primitives (Avatar, Modals, etc.)
├── context/              # React contexts (all business logic here)
├── hooks/                # Custom hooks (state changes here)
├── lib/                  # Utilities (config, WASM codec, formatters)
├── models/               # TypeScript interfaces (*.model.ts)
├── types/                # TypeScript types (*.types.ts)
├── worker/               # AudioWorklet + analyzer worker sources
├── assets/               # Static assets (logos, images)
└── pkg/                  # Compiled WASM output (core-wasm)
```

## Audio Pipeline

```mermaid
flowchart LR
    MIC["🎙 Microphone<br/>getUserMedia()"]
    SRC["MediaStreamSource"]
    AWN["AudioWorkletNode<br/>noise-gate-processor"]

    subgraph WASM["WASM in AudioWorklet"]
        RNN["RNNoise<br/>Deep noise suppression"]
        SG["SmartGate<br/>VAD / fixed threshold"]
        TS["TransientSuppressor<br/>Keyboard click removal"]
    end

    DST["MediaStreamDestination"]
    RTC["RTCPeerConnection → SFU"]

    MIC --> SRC --> AWN
    AWN --> RNN --> SG --> TS
    TS --> AWN
    AWN --> DST --> RTC
```

## Conventions

- **Components** must remain stateless and agnostic — no direct API calls
- **Business logic** lives exclusively in `context/`
- **State mutations** happen in `hooks/`
- **Interfaces** in `models/` (`*.model.ts`), **types** in `types/` (`*.types.ts`)
- **Max 350 lines** per file — extract logic if exceeded
- **TailwindCSS v4** for styling, **lucide-react** for icons
- **Comments** in English, JSDoc format

## Direct Messages (1-to-1)

DMs travel on the **same authenticated WebSocket** as friends/server presence — they do **not** open a second socket and do **not** share the WebRTC media transport. The voice/video path stays untouched: media flows over WebRTC peer-connections through the SFU, while DMs ride the JSON control channel.

```mermaid
sequenceDiagram
    participant UI as FriendAvatar
    participant CTX as DmContext
    participant WS as Signaling WS
    participant SRV as signaling-server

    UI->>CTX: openDm(friend) (left-click)
    CTX->>SRV: rpc("dm.history", { userId })
    SRV-->>CTX: DmEntry[]
    UI->>CTX: sendDm("hi")
    CTX->>CTX: append optimistic placeholder (pending=true)
    CTX->>WS: { type:"dm-send", toUserId, message, clientMsgId }
    SRV-->>WS: { type:"dm-message", … } (echo + recipient)
    SRV-->>WS: { type:"dm-ack", id, clientMsgId }
    WS-->>CTX: useDmRealtime resolves the placeholder
```

Files:
- `context/DmContext.tsx` — opens/closes/sends conversations, optimistic placeholders.
- `hooks/useDmRealtime.ts` — bus subscription that mutates the conversation map.
- `api/dm.ws.ts` — `sendDmWs`, `fetchDmHistory`, `fetchDmPartners` (all WebSocket, no REST).
- `components/dm/` — `DmPanel`, `DmMessageList`, `DmComposer`, `DmTabs`.
- `components/friends/FriendContextMenu.tsx` — right-click on a friend → *Send a message* / *Remove*.

Friend removal is **bilateral** (single row in the server's `friends` store) and the other party is notified live via `friend-removed` over the same auth-keyed WS registry — no refresh required.

## Scripts

```sh
pnpm dev              # Start Vite dev server (port 1420)
pnpm build            # TypeScript check + Vite production build
pnpm build:worklet    # Compile AudioWorklet to public/worker/
pnpm wasm:build       # Compile core-wasm → src/pkg/
pnpm tauri            # Run Tauri CLI
```

## License

**BSL-1.1** — See [LICENSE](../../LICENSE).

---

# Void — Frontend React (FR)

Frontend **React 19** + **Vite 7** + **TypeScript** avec **TailwindCSS v4**. Architecture stricte : composants muets, logique métier dans les contexts, changements d'état dans les hooks.

## Architecture

```mermaid
graph TB
    COMPONENTS["<b>Composants (UI muette)</b><br/>auth · channel · chat · layout<br/>settings · sidebar · stream · ui"]
    CONTEXTS["<b>Contexts (Logique Métier)</b><br/>Auth · Voice · Stream · Chat<br/>Dm · Server · Toast · Bento"]
    HOOKS["<b>Hooks (Logique d'État)</b><br/>useDashboardState · useBentoLayout<br/>useNetworkStats · usePushToTalk<br/>useVoiceActivity · useProfileSettings"]
    API["<b>Couche API</b><br/>http-client · auth.api<br/>friends.ws · signalingBus"]
    WORKERS["<b>Workers (Hors thread principal)</b><br/>noise-gate.worklet · analyzer.worker"]

    COMPONENTS --> CONTEXTS --> HOOKS
    HOOKS --> API
    HOOKS --> WORKERS
```

*Chaque bloc liste ses sous-modules — voir [Structure des Fichiers](#structure-des-fichiers) ci-dessous pour le détail complet.*

## Structure des Fichiers

```
src/
├── api/                  # Client HTTP + modules d'endpoints
├── components/           # Composants UI muets et agnostiques
│   ├── auth/             # Écran de connexion
│   ├── channel/          # Liste de channels, items, modal de création
│   ├── chat/             # Panel de chat
│   ├── layout/           # Layout principal, sidebar, barre de titre
│   ├── settings/         # Panels de paramètres (voix, profil, etc.)
│   ├── sidebar/          # Sidebar serveur, footer utilisateur, membres
│   ├── stream/           # Cartes de stream, renderer audio
│   └── ui/               # Primitives UI partagées (Avatar, Modals, etc.)
├── context/              # Contexts React (toute la logique métier ici)
├── hooks/                # Hooks custom (changements d'état ici)
├── lib/                  # Utilitaires (config, codec WASM, formateurs)
├── models/               # Interfaces TypeScript (*.model.ts)
├── types/                # Types TypeScript (*.types.ts)
├── worker/               # Sources AudioWorklet + worker d'analyse
├── assets/               # Ressources statiques (logos, images)
└── pkg/                  # Sortie WASM compilée (core-wasm)
```

## Pipeline Audio

```mermaid
flowchart LR
    MIC["🎙 Microphone<br/>getUserMedia()"]
    SRC["MediaStreamSource"]
    AWN["AudioWorkletNode<br/>noise-gate-processor"]

    subgraph WASM["WASM dans le AudioWorklet"]
        RNN["RNNoise<br/>Suppression de bruit profonde"]
        SG["SmartGate<br/>VAD / seuil fixe"]
        TS["TransientSuppressor<br/>Suppression clics clavier"]
    end

    DST["MediaStreamDestination"]
    RTC["RTCPeerConnection → SFU"]

    MIC --> SRC --> AWN
    AWN --> RNN --> SG --> TS
    TS --> AWN
    AWN --> DST --> RTC
```

## Conventions

- Les **composants** doivent rester stateless et agnostiques — pas d'appels API directs
- La **logique métier** vit exclusivement dans `context/`
- Les **mutations d'état** se font dans `hooks/`
- Les **interfaces** dans `models/` (`*.model.ts`), les **types** dans `types/` (`*.types.ts`)
- **350 lignes max** par fichier — extraire la logique si dépassé
- **TailwindCSS v4** pour le style, **lucide-react** pour les icônes
- **Commentaires** en anglais, format JSDoc

## Scripts

```sh
pnpm dev              # Lancer le serveur dev Vite (port 1420)
pnpm build            # Vérification TypeScript + build production Vite
pnpm build:worklet    # Compiler l'AudioWorklet dans public/worker/
pnpm wasm:build       # Compiler core-wasm → src/pkg/
pnpm tauri            # Lancer le CLI Tauri
```

## Licence

**BSL-1.1** — Voir [LICENSE](../../LICENSE).
