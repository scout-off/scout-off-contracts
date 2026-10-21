import json
import subprocess
import os

def run_cargo():
    result = subprocess.run(['cargo', 'test', '--no-run', '--message-format=json'], capture_output=True, text=True)
    return result.stdout.split('\n')

lines = run_cargo()
fixes = []

for line in lines:
    if not line: continue
    try:
        msg = json.loads(line)
        if msg.get('reason') == 'compiler-message':
            message = msg['message']
            if message['code'] and message['code']['code'] == 'E0061':
                # Missing argument
                spans = message['spans']
                for span in spans:
                    if span['is_primary']:
                        file_name = span['file_name']
                        line_num = span['line_end']
                        col_num = span['column_end']
                        fixes.append((file_name, line_num, col_num))
    except Exception as e:
        pass

# Group fixes by file and apply them from bottom to top (to not mess up line numbers)
fixes = sorted(list(set(fixes)), key=lambda x: (x[0], -x[1], -x[2]))

for fix in fixes:
    file_name, line_num, col_num = fix
    if not os.path.exists(file_name): continue
    
    with open(file_name, 'r') as f:
        lines = f.readlines()
        
    line_idx = line_num - 1
    line = lines[line_idx]
    
    # insert `, &soroban_sdk::String::from_str(&env, "DefaultAffiliation")` at col_num
    # Wait, the column might be the closing parenthesis `)`
    # Let's inspect the character at col_num - 1
    # Actually, E0061 span might highlight the entire function call `client.register_validator(...)`
    # Let's just do a string replacement on the line if it has register_validator
    pass
