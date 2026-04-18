import { helper } from "./utils/helper";

export async function processRequest(req: Request): Promise<void> {
  helper(req);
}
