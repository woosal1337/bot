# 0016 — Save effort by provider and model

Status: Accepted

Date: 2026-09-16

## Decision

Bot saves an optional effort preference for each provider and model. The effort value is canonical, but the picker uses labels and choices from the provider's model catalog. A new conversation applies the saved value before it sends a queued prompt. A command-line effort choice takes priority. Resumed conversations keep their saved effort.

## Reason

Models do not offer the same effort levels. One global value could send a level that a model does not support or change another provider's behavior. Bot stores only a user preference. The provider still owns its account, thread, and model protocol.

## Result

Changing the default does not change an open conversation. If a model stops offering a saved level, Bot uses that model's provider default. Users can clear an override with “Model default.”
