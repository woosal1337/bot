import json
import sys


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def result(request_id, value):
    send({"jsonrpc": "2.0", "id": request_id, "result": value})


def read_message():
    line = sys.stdin.readline()
    if not line:
        raise EOFError
    return json.loads(line)


def elicit():
    request_id = 9001
    send(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "elicitation/create",
            "params": {
                "message": "Enter the Bot MCP verification value.",
                "requestedSchema": {
                    "type": "object",
                    "properties": {
                        "value": {
                            "type": "string",
                            "title": "Verification value",
                        }
                    },
                    "required": ["value"],
                },
                "_meta": {"trace": "bot-live-mcp"},
            },
        }
    )
    while True:
        message = read_message()
        if message.get("id") == request_id and "result" in message:
            return message["result"]
        if message.get("method") == "ping" and "id" in message:
            result(message["id"], {})


def handle(message):
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        result(
            request_id,
            {
                "protocolVersion": message["params"]["protocolVersion"],
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {
                    "name": "bot-elicitation-fixture",
                    "version": "1.0.0",
                },
            },
        )
    elif method == "tools/list":
        result(
            request_id,
            {
                "tools": [
                    {
                        "name": "request_value",
                        "description": "Request the Bot MCP verification value.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": False,
                        },
                    }
                ]
            },
        )
    elif method == "tools/call":
        response = elicit()
        value = response.get("content", {}).get("value", "")
        result(
            request_id,
            {
                "content": [
                    {
                        "type": "text",
                        "text": f"Accepted verification value: {value}",
                    }
                ],
                "isError": response.get("action") != "accept",
            },
        )
    elif method == "ping":
        result(request_id, {})


def main():
    while True:
        try:
            handle(read_message())
        except EOFError:
            return


main()
