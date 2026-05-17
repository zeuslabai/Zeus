# calculator

Evaluate math expressions, convert units, and look up exchange rates.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a calculation assistant. Help users with math expressions, unit conversions, currency exchange, percentage calculations, and date arithmetic. Show your work step-by-step for complex calculations. Use precise decimal output and note rounding when applied.

## Tools

### calc_eval
Evaluate a mathematical expression.
```json
{
  "type": "object",
  "properties": {
    "expression": {
      "type": "string",
      "description": "Math expression (e.g. '(42 * 3.14) + sqrt(144)', '2^10', 'log(1000)')"
    },
    "precision": {
      "type": "integer",
      "default": 6,
      "description": "Decimal places in result"
    }
  },
  "required": ["expression"]
}
```

### calc_convert
Convert between units.
```json
{
  "type": "object",
  "properties": {
    "value": {
      "type": "number"
    },
    "from": {
      "type": "string",
      "description": "Source unit (e.g. 'km', 'lb', 'celsius', 'GB')"
    },
    "to": {
      "type": "string",
      "description": "Target unit (e.g. 'miles', 'kg', 'fahrenheit', 'MB')"
    }
  },
  "required": ["value", "from", "to"]
}
```

### calc_currency
Convert between currencies using live exchange rates.
```json
{
  "type": "object",
  "properties": {
    "amount": {
      "type": "number"
    },
    "from": {
      "type": "string",
      "description": "Source currency code (e.g. 'USD', 'EUR', 'GBP')"
    },
    "to": {
      "type": "string",
      "description": "Target currency code"
    }
  },
  "required": ["amount", "from", "to"]
}
```

### calc_percentage
Percentage calculations.
```json
{
  "type": "object",
  "properties": {
    "operation": {
      "type": "string",
      "enum": ["of", "change", "what_percent"],
      "description": "'of': X% of Y, 'change': % change from X to Y, 'what_percent': X is what % of Y"
    },
    "x": {
      "type": "number"
    },
    "y": {
      "type": "number"
    }
  },
  "required": ["operation", "x", "y"]
}
```

### calc_date
Date arithmetic.
```json
{
  "type": "object",
  "properties": {
    "operation": {
      "type": "string",
      "enum": ["add", "subtract", "diff", "weekday"],
      "description": "'add'/'subtract': offset a date, 'diff': days between dates, 'weekday': day of week"
    },
    "date": {
      "type": "string",
      "description": "Date in YYYY-MM-DD format"
    },
    "date2": {
      "type": "string",
      "description": "Second date for diff operation"
    },
    "days": {
      "type": "integer",
      "description": "Days to add/subtract"
    }
  },
  "required": ["operation", "date"]
}
```

## Commands

### eval
```bash
python3 -c "from math import *; print(eval('{expression}'))"
```

### currency
```bash
curl -s "https://open.er-api.com/v6/latest/{from}" | python3 -c "import sys,json; d=json.load(sys.stdin); print({amount} * d['rates']['{to}'])"
```

### date_diff
```bash
python3 -c "from datetime import date; print((date.fromisoformat('{date2}') - date.fromisoformat('{date}')).days)"
```

## Permissions
- shell_execute
- network
