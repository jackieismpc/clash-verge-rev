import clc from 'cli-color'

export const log_success = (msg, ...optionalParams) =>
  console.log(clc.green(msg), ...optionalParams)
export const log_info = (msg, ...optionalParams) =>
  console.log(clc.bgBlue(msg), ...optionalParams)
