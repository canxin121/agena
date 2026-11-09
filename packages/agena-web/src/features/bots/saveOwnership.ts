/** A pending save can update the catalog after dismissal, but may not change navigation. */
export class SaveOwnership {
  cancelled = false
  private navigationVersion: number
  private location: string
  private windowId: string
  constructor(navigationVersion: number, location: string, windowId: string) {
    this.navigationVersion = navigationVersion
    this.location = location
    this.windowId = windowId
  }
  cancel() {
    this.cancelled = true
  }
  canNavigate(navigationVersion: number, location: string, windowId: string) {
    return (
      !this.cancelled &&
      this.navigationVersion === navigationVersion &&
      this.location === location &&
      this.windowId === windowId
    )
  }
}
