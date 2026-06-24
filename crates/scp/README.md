# Schooner communication protocol

This is a server to which you can connect in order to communicate with the engine internals.

Works in a similar way to LSP, but communication is carried out over the network, with HTTP/WebSocket(future).

## Capabilities

The server starting together with the game and listen port 3030.

It accept prompt commands in **morse language** in a following way:

```json
{
    "type": "cmd",
    "request": "let cube (find.enitity 32)\nmove.right $cube 2"
}
```

Request would be interpretated on the server side, would get the required information andapply transformations, and then respond:

```json
{
    "transaction_id": 1,
    "type": "cmd",
    "state": { "variables": ["$cube"] }, /* list of available variables */
    "body": { "components": { ...state of components... } }
}
```

### Notifications to the client

By using long polling requests, you can get notification from the server on the client side.

For example, you enter the debug mode in the game and select the entity for inspection. The server will send a following request(response to the long poll listener):


```json
{
    "transaction_id": 10,
    "type": "target_selected",
    "state": { "variables": [..., "$target"] },
    "body": {}
}
```

## Client

Client can be any who can speak in the protocol, but the original client is CLI rust app in the `src/client/`, which supports pretty printing, knows about every command, support autocompletion (for known variables and identifiers), and navigation with emacs shortcuts

## Connection

Server provides a special `system` which can be embed into the game world. You also need to provide a special `resource` with the configuration of the server (for example, some preloaded for the current scene entities in variables, or anything else)

## Morsa language

Very primitive command line language. Everything done as a function application, left to  right (Lisp notation with Haskell syntax). Supports variable declaration and paranthesis for explicit grouping.

### Primitives

- Number
- Identifier
- Bool

### Variables

Variables can be defined inside the language, or be provided from the server side.

### Example

Arithmetics

```
let x 10
let y 30
let result (+ $x $y)
```

```
move.right $cube 20
```

## Connection with the engine


## Distilated for release

Nothing related to the `scp` or its dependencies is propagated to the release binary of the game, neither it starts the server. That is dev only capability.

