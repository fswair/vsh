# Baseline comparison

- baseline: `playground/reports/baseline-pre-roadmap-20260609-193413`
- current: `playground/reports/current-20260609-235916`
- overall: **FAIL**

## Playground ratio diff (current / baseline)

| command | mode | baseline | current | delta | regressed |
|---------|------|---------:|--------:|------:|:---------:|
| cat | vsh_apply | 0.022x | 0.025x | +0.003x | False |
| cat | vsh_full | 0.159x | 0.382x | +0.223x | True |
| cd | vsh_apply | 0.023x | 0.028x | +0.005x | False |
| cd | vsh_full | 0.200x | 0.467x | +0.267x | True |
| chmod | vsh_apply | 0.020x | 0.023x | +0.003x | False |
| chmod | vsh_full | 0.289x | 0.514x | +0.225x | True |
| cp | vsh_apply | 0.066x | 0.082x | +0.015x | False |
| cp | vsh_full | 0.330x | 0.541x | +0.211x | True |
| du | vsh_apply | 0.050x | 0.055x | +0.005x | False |
| du | vsh_full | 0.268x | 0.440x | +0.172x | True |
| echo | vsh_apply | 0.001x | 0.000x | -0.000x | False |
| echo | vsh_full | 0.202x | 0.427x | +0.224x | True |
| echo_write | vsh_apply | 0.045x | 0.051x | +0.006x | False |
| echo_write | vsh_full | 0.418x | 0.701x | +0.283x | True |
| find | vsh_apply | 0.108x | 0.135x | +0.027x | False |
| find | vsh_full | 0.269x | 0.520x | +0.251x | True |
| grep | vsh_apply | 0.175x | 0.201x | +0.026x | False |
| grep | vsh_full | 0.337x | 0.533x | +0.196x | True |
| head | vsh_apply | 0.022x | 0.026x | +0.004x | False |
| head | vsh_full | 0.145x | 0.380x | +0.235x | True |
| ln | vsh_apply | 0.047x | 0.052x | +0.005x | False |
| ln | vsh_full | 0.344x | 0.540x | +0.195x | True |
| ls | vsh_apply | 0.026x | 0.027x | +0.001x | False |
| ls | vsh_full | 0.160x | 0.331x | +0.171x | True |
| mkdir | vsh_apply | 0.040x | 0.041x | +0.001x | False |
| mkdir | vsh_full | 0.335x | 0.565x | +0.230x | True |
| mv | vsh_apply | 0.058x | 0.059x | +0.000x | False |
| mv | vsh_full | 0.379x | 0.559x | +0.181x | True |
| nl | vsh_apply | 0.025x | 0.026x | +0.001x | False |
| nl | vsh_full | 0.154x | 0.351x | +0.197x | True |
| pwd | vsh_apply | 0.000x | 0.000x | -0.000x | False |
| pwd | vsh_full | 0.187x | 0.407x | +0.219x | True |
| rg | vsh_apply | 0.135x | 0.134x | -0.001x | False |
| rg | vsh_full | 0.235x | 0.335x | +0.099x | False |
| rm | vsh_apply | 0.025x | 0.029x | +0.003x | False |
| rm | vsh_full | 0.269x | 0.477x | +0.208x | True |
| sed | vsh_apply | 0.024x | 0.025x | +0.001x | False |
| sed | vsh_full | 0.169x | 0.328x | +0.159x | True |
| sed_inplace | vsh_apply | 0.032x | 0.032x | +0.001x | False |
| sed_inplace | vsh_full | 0.293x | 0.483x | +0.190x | True |
| sort | vsh_apply | 0.024x | 0.025x | +0.001x | False |
| sort | vsh_full | 0.183x | 0.358x | +0.175x | True |
| stat | vsh_apply | 0.014x | 0.014x | +0.000x | False |
| stat | vsh_full | 0.099x | 0.216x | +0.116x | True |
| tail | vsh_apply | 0.025x | 0.025x | -0.000x | False |
| tail | vsh_full | 0.183x | 0.340x | +0.157x | True |
| touch | vsh_apply | 0.033x | 0.032x | -0.001x | False |
| touch | vsh_full | 0.345x | 0.536x | +0.191x | True |
| wc | vsh_apply | 0.025x | 0.024x | -0.000x | False |
| wc | vsh_full | 0.188x | 0.341x | +0.153x | True |

## Agent context diff

| metric | baseline | current | delta |
|--------|---------:|--------:|------:|
| native_duration_ms | 9254.230708 | 8410.481209 | -843.7 |
| native_input_tokens | 4135 | 4127.0 | -8.0 |
| native_tool_calls | 5 | 5.0 | +0.0 |
| native_validation_passed | True | True | +0.0 |
| vsh_duration_ms | 12253.864125 | 8151.210334 | -4102.7 |
| vsh_input_tokens | 7359 | 3416.0 | -3943.0 |
| vsh_tool_calls | 5 | 3.0 | -2.0 |
| vsh_validation_passed | True | True | +0.0 |