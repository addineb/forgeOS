#!/usr/bin/env python3
"""Debug date extraction."""
ts = 1764547499914000000
secs = ts // 1_000_000_000
days = secs // 86400
print(f"ts={ts}")
print(f"secs={secs}")
print(f"days={days}")

def days_to_ymd(days):
    days += 719468
    era = days // 146097 if days >= 0 else (days - 146096) // 146097
    day_of_era = days - era * 146097
    year_of_era = (day_of_era - day_of_era // 1460 + day_of_era // 36524 - day_of_era // 146096) // 365
    day_of_year = day_of_era - (365 * year_of_era + year_of_era // 4 - year_of_era // 100)
    year = year_of_era + era * 400
    doy = day_of_year + 1
    if doy > 365 + (1 if year % 4 == 0 and (year % 100 != 0 or year % 400 == 0) else 0):
        year += 1
        doy = 1
    months = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    if year % 4 == 0 and (year % 100 != 0 or year % 400 == 0):
        months[1] = 29
    month = 1
    while doy > months[month - 1]:
        doy -= months[month - 1]
        month += 1
    day = doy
    return year, month, day

y, m, d = days_to_ymd(days)
print(f"date={y:04}-{m:02}-{d:02}")
