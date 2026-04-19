"""Launcher that registers manual pricing in LiteLLM before running sweagent.

SWE-agent calls `litellm.cost_calculator.completion_cost(response)` after
every API call. When the model isn't in LiteLLM's baked-in `model_cost`
table (M2.7, Opus 4.7, most 2026 paid models), that call raises and
SWE-agent bails with ModelConfigurationError. Passing pricing via
`completion_kwargs` only affects `litellm.completion()`, not the cost
calculation.

Fix: register our manual prices in-process via `litellm.register_model()`
before sweagent's subprocess starts making calls. This runs in the same
interpreter as sweagent, so the registration persists for the whole run.
"""

from __future__ import annotations

import sys

# Per-token USD pricing. Keep in sync with
# bench/sweagent_runner.py::MODEL_PRICING_USD_PER_TOKEN.
PRICING = {
    "openrouter/minimax/minimax-m2.7": (0.30e-6, 1.20e-6),
    "openrouter/minimax/minimax-m2.5": (0.30e-6, 1.20e-6),
    "openrouter/minimax/minimax-m2.1": (0.30e-6, 1.20e-6),
    "openrouter/z-ai/glm-4.6": (0.15e-6, 0.60e-6),
    "openrouter/z-ai/glm-4.5-air": (0.075e-6, 0.30e-6),
    "openrouter/anthropic/claude-opus-4-7": (15.0e-6, 75.0e-6),
    "openrouter/anthropic/claude-sonnet-4-6": (3.0e-6, 15.0e-6),
}


def _register() -> None:
    import litellm

    model_cost_entries = {}
    for name, (in_cost, out_cost) in PRICING.items():
        # Strip the "openrouter/" prefix for the LiteLLM lookup key — its
        # cost table is keyed by the inner provider/model pair, not the
        # routing prefix.
        bare = name.removeprefix("openrouter/")
        model_cost_entries[bare] = {
            "input_cost_per_token": in_cost,
            "output_cost_per_token": out_cost,
            "litellm_provider": "openrouter",
            "mode": "chat",
            "max_tokens": 8192,
            "max_input_tokens": 200000,
            "max_output_tokens": 8192,
            "supports_function_calling": True,
        }
        # Also register under the routed name — LiteLLM sometimes looks
        # up the full "openrouter/..." string.
        model_cost_entries[name] = dict(model_cost_entries[bare])

    litellm.register_model(model_cost_entries)


def main() -> int:
    _register()
    from sweagent.run.run import main as sweagent_main

    return sweagent_main()


if __name__ == "__main__":
    raise SystemExit(main())
